// C-шим над libavcodec для спайка video_loop:
// кодирование H.264 (h264_qsv / libx264) и декодирование H.264.
// Вся работа со структурами ffmpeg — на стороне C, Rust получает простой API.
#include <libavcodec/avcodec.h>
#include <libavutil/opt.h>
#include <libavutil/imgutils.h>
#include <libavutil/error.h>
#include <libswscale/swscale.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

static void put_err(char* err, int cap, const char* msg)
{
    if (err && cap > 0) {
        snprintf(err, (size_t)cap, "%s", msg);
    }
}

static void put_err_av(char* err, int cap, const char* prefix, int code)
{
    if (err && cap > 0) {
        char buf[256];
        av_strerror(code, buf, sizeof(buf));
        snprintf(err, (size_t)cap, "%s: %s (%d)", prefix, buf, code);
    }
}

static int has_opt(void* obj, const char* name)
{
    return av_opt_find(obj, name, NULL, 0, 0) != NULL;
}

/* ============================== ENCODER ============================== */

typedef struct RlEnc {
    AVCodecContext* ctx;
    AVFrame* copy_frame; /* внутренняя копия входящего кадра (lifetime-safe) */
    AVPacket* pkt;
    int w, h;
} RlEnc;

RlEnc* rl_enc_create(const char* codec_name, int w, int h, int fps, int64_t bitrate,
                     int gop, const char* preset, int async_depth, int low_delay,
                     char* err, int err_cap)
{
    const AVCodec* codec = avcodec_find_encoder_by_name(codec_name);
    if (!codec) {
        put_err(err, err_cap, "encoder not found");
        return NULL;
    }

    RlEnc* e = (RlEnc*)calloc(1, sizeof(RlEnc));
    if (!e) {
        put_err(err, err_cap, "calloc failed");
        return NULL;
    }
    e->w = w;
    e->h = h;

    e->ctx = avcodec_alloc_context3(codec);
    e->copy_frame = av_frame_alloc();
    e->pkt = av_packet_alloc();
    if (!e->ctx || !e->copy_frame || !e->pkt) {
        put_err(err, err_cap, "alloc failed");
        goto fail;
    }

    e->ctx->width = w;
    e->ctx->height = h;
    e->ctx->time_base = (AVRational){1, fps};
    e->ctx->framerate = (AVRational){fps, 1};
    e->ctx->pix_fmt = AV_PIX_FMT_NV12;
    e->ctx->bit_rate = bitrate;
    e->ctx->rc_max_rate = bitrate;
    e->ctx->rc_buffer_size = (int)(bitrate / 2);
    e->ctx->gop_size = gop;
    e->ctx->max_b_frames = 0;
    e->ctx->thread_count = 1;
    e->ctx->flags |= AV_CODEC_FLAG_LOW_DELAY;

    if (preset && preset[0] && has_opt(e->ctx->priv_data, "preset")) {
        av_opt_set(e->ctx->priv_data, "preset", preset, 0);
    }
    if (async_depth >= 0 && has_opt(e->ctx->priv_data, "async_depth")) {
        char v[16];
        snprintf(v, sizeof(v), "%d", async_depth);
        av_opt_set(e->ctx->priv_data, "async_depth", v, 0);
    }
    if (low_delay >= 0 && has_opt(e->ctx->priv_data, "low_delay_brc")) {
        av_opt_set_int(e->ctx->priv_data, "low_delay_brc", low_delay, 0);
    }
    if (strstr(codec_name, "x264") != NULL && has_opt(e->ctx->priv_data, "tune")) {
        av_opt_set(e->ctx->priv_data, "tune", "zerolatency", 0);
    }

    int ret = avcodec_open2(e->ctx, codec, NULL);
    if (ret < 0) {
        put_err_av(err, err_cap, "avcodec_open2", ret);
        goto fail;
    }
    return e;

fail:
    if (e->pkt) av_packet_free(&e->pkt);
    if (e->copy_frame) av_frame_free(&e->copy_frame);
    if (e->ctx) avcodec_free_context(&e->ctx);
    free(e);
    return NULL;
}

/* Возвращает: 0 = приняли; 1 = нужно сначала забрать пакет (EAGAIN); <0 ошибка */
int rl_enc_send(RlEnc* e, const uint8_t* y, int y_stride, const uint8_t* uv, int uv_stride,
                int64_t pts, char* err, int err_cap)
{
    if (!e) {
        put_err(err, err_cap, "null encoder");
        return -1;
    }

    AVFrame* f = e->copy_frame;
    if (f->data[0] == NULL || f->width != e->w || f->height != e->h) {
        f->format = AV_PIX_FMT_NV12;
        f->width = e->w;
        f->height = e->h;
        int ret = av_frame_get_buffer(f, 32);
        if (ret < 0) {
            put_err_av(err, err_cap, "av_frame_get_buffer", ret);
            return -2;
        }
    }
    int ret = av_frame_make_writable(f);
    if (ret < 0) {
        put_err_av(err, err_cap, "av_frame_make_writable", ret);
        return -2;
    }

    for (int j = 0; j < e->h; j++) {
        memcpy(f->data[0] + (size_t)j * f->linesize[0], y + (size_t)j * y_stride, (size_t)e->w);
    }
    for (int j = 0; j < e->h / 2; j++) {
        memcpy(f->data[1] + (size_t)j * f->linesize[1], uv + (size_t)j * uv_stride, (size_t)e->w);
    }
    f->pts = pts;

    ret = avcodec_send_frame(e->ctx, f);
    if (ret == AVERROR(EAGAIN)) {
        return 1;
    }
    if (ret < 0) {
        put_err_av(err, err_cap, "send_frame", ret);
        return -3;
    }
    return 0;
}

/* Возвращает: 1 = есть пакет (out_size), 0 = пока нет, <0 ошибка */
int rl_enc_recv(RlEnc* e, uint8_t* out, int out_cap, int* out_size, char* err, int err_cap)
{
    if (!e) {
        return -1;
    }
    int ret = avcodec_receive_packet(e->ctx, e->pkt);
    if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF) {
        return 0;
    }
    if (ret < 0) {
        put_err_av(err, err_cap, "receive_packet", ret);
        return -2;
    }
    if (e->pkt->size > out_cap) {
        put_err(err, err_cap, "packet larger than buffer");
        av_packet_unref(e->pkt);
        return -3;
    }
    memcpy(out, e->pkt->data, (size_t)e->pkt->size);
    *out_size = e->pkt->size;
    av_packet_unref(e->pkt);
    return 1;
}

void rl_enc_free(RlEnc* e)
{
    if (!e) {
        return;
    }
    if (e->pkt) av_packet_free(&e->pkt);
    if (e->copy_frame) av_frame_free(&e->copy_frame);
    if (e->ctx) avcodec_free_context(&e->ctx);
    free(e);
}

/* ============================== DECODER ============================== */

typedef struct RlDec {
    AVCodecContext* ctx;
    AVFrame* frame;
    AVPacket* pkt;
    int got_frame;
} RlDec;

RlDec* rl_dec_create(char* err, int err_cap)
{
    const AVCodec* codec = avcodec_find_decoder(AV_CODEC_ID_H264);
    if (!codec) {
        put_err(err, err_cap, "h264 decoder not found");
        return NULL;
    }
    RlDec* d = (RlDec*)calloc(1, sizeof(RlDec));
    if (!d) {
        put_err(err, err_cap, "calloc failed");
        return NULL;
    }
    d->ctx = avcodec_alloc_context3(codec);
    d->frame = av_frame_alloc();
    d->pkt = av_packet_alloc();
    if (!d->ctx || !d->frame || !d->pkt) {
        put_err(err, err_cap, "alloc failed");
        goto fail;
    }
    d->ctx->thread_count = 1;
    d->ctx->flags |= AV_CODEC_FLAG_LOW_DELAY;
    d->ctx->flags2 |= AV_CODEC_FLAG2_FAST;

    int ret = avcodec_open2(d->ctx, codec, NULL);
    if (ret < 0) {
        put_err_av(err, err_cap, "decoder open", ret);
        goto fail;
    }
    return d;

fail:
    if (d->pkt) av_packet_free(&d->pkt);
    if (d->frame) av_frame_free(&d->frame);
    if (d->ctx) avcodec_free_context(&d->ctx);
    free(d);
    return NULL;
}

/* Возвращает: 0 = приняли; 1 = нужно сначала забрать кадр (EAGAIN); <0 ошибка */
int rl_dec_send(RlDec* d, const uint8_t* data, int size, char* err, int err_cap)
{
    if (!d) {
        return -1;
    }
    av_packet_unref(d->pkt);
    int ret = av_new_packet(d->pkt, size);
    if (ret < 0) {
        put_err_av(err, err_cap, "av_new_packet", ret);
        return -2;
    }
    memcpy(d->pkt->data, data, (size_t)size);
    ret = avcodec_send_packet(d->ctx, d->pkt);
    if (ret == AVERROR(EAGAIN)) {
        return 1;
    }
    if (ret < 0) {
        put_err_av(err, err_cap, "send_packet", ret);
        return -3;
    }
    return 0;
}

/* Возвращает: 1 = кадр готов, 0 = нет, <0 ошибка */
int rl_dec_recv(RlDec* d, char* err, int err_cap)
{
    if (!d) {
        return -1;
    }
    int ret = avcodec_receive_frame(d->ctx, d->frame);
    if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF) {
        d->got_frame = 0;
        return 0;
    }
    if (ret < 0) {
        put_err_av(err, err_cap, "receive_frame", ret);
        return -2;
    }
    d->got_frame = 1;
    return 1;
}

/* Прямой доступ к плоскостям текущего кадра (0=Y, 1=U, 2=V). NULL если кадра нет. */
const uint8_t* rl_dec_plane(RlDec* d, int idx, int* stride, int* w, int* h)
{
    if (!d || !d->got_frame) {
        return NULL;
    }
    if (idx == 0) {
        if (stride) *stride = d->frame->linesize[0];
        if (w) *w = d->frame->width;
        if (h) *h = d->frame->height;
        return d->frame->data[0];
    }
    if (idx == 1) {
        if (stride) *stride = d->frame->linesize[1];
        if (w) *w = d->frame->width / 2;
        if (h) *h = d->frame->height / 2;
        return d->frame->data[1];
    }
    if (stride) *stride = d->frame->linesize[2];
    if (w) *w = d->frame->width / 2;
    if (h) *h = d->frame->height / 2;
    return d->frame->data[2];
}

/* Числовой код формата пикселей текущего кадра (для проверки, что это yuv420p). */
int rl_dec_pix_fmt(RlDec* d)
{
    if (!d || !d->got_frame) {
        return -1;
    }
    return (int)d->frame->format;
}

void rl_dec_free(RlDec* d)
{
    if (!d) {
        return;
    }
    if (d->pkt) av_packet_free(&d->pkt);
    if (d->frame) av_frame_free(&d->frame);
    if (d->ctx) avcodec_free_context(&d->ctx);
    free(d);
}

/* ============================== SWS (конверсии) ============================== */

/* which: 0=BGRA, 1=NV12, 2=YUV420P, 3=RGBA (числовые значения AVPixelFormat не стабильны между версиями) */
int rl_pix_fmt(int which)
{
    switch (which) {
    case 0: return (int)AV_PIX_FMT_BGRA;
    case 1: return (int)AV_PIX_FMT_NV12;
    case 2: return (int)AV_PIX_FMT_YUV420P;
    case 3: return (int)AV_PIX_FMT_RGBA;
    default: return -1;
    }
}

void* rl_sws_create(int w, int h, int src_fmt, int dst_fmt)
{
    struct SwsContext* ctx = sws_getContext(w, h, (enum AVPixelFormat)src_fmt,
                                            w, h, (enum AVPixelFormat)dst_fmt,
                                            SWS_BILINEAR, NULL, NULL, NULL);
    return ctx;
}

int rl_sws_bgra_to_nv12(void* ctx, const uint8_t* bgra, int bgra_stride,
                        uint8_t* y, int y_stride, uint8_t* uv, int uv_stride, int h)
{
    if (!ctx) return -1;
    const uint8_t* src[4] = { bgra, NULL, NULL, NULL };
    int src_stride[4] = { bgra_stride, 0, 0, 0 };
    uint8_t* dst[4] = { y, uv, NULL, NULL };
    int dst_stride[4] = { y_stride, uv_stride, 0, 0 };
    return sws_scale((struct SwsContext*)ctx, src, src_stride, 0, h, dst, dst_stride);
}

int rl_sws_yuv_to_rgba(void* ctx, const uint8_t* y, int ys, const uint8_t* u, int us,
                       const uint8_t* v, int vs, uint8_t* rgba, int rgba_stride, int h)
{
    if (!ctx) return -1;
    const uint8_t* src[4] = { y, u, v, NULL };
    int src_stride[4] = { ys, us, vs, 0 };
    uint8_t* dst[4] = { rgba, NULL, NULL, NULL };
    int dst_stride[4] = { rgba_stride, 0, 0, 0 };
    return sws_scale((struct SwsContext*)ctx, src, src_stride, 0, h, dst, dst_stride);
}

void rl_sws_free(void* ctx)
{
    if (ctx) sws_freeContext((struct SwsContext*)ctx);
}
