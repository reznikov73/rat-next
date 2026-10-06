//! Превью форм курсоров: рендерит все виды на белом/чёрном/сером фоне через
//! штатный тесселятор egui и сохраняет PPM (открыть любым просмотрщиком).
//!
//! Запуск: cargo run -p rat-cursor-ui --example preview

use std::io::Write;

use egui::epaint::{ClippedPrimitive, Primitive};
use egui::{Color32, Context, FontDefinitions, Pos2, RawInput, Rect, pos2, vec2};

use rat_cursor_ui::paint_cursor;

const CELL_W: usize = 280;
const CELL_H: usize = 260;
const SCALE: f32 = 5.0;

struct Canvas {
    w: usize,
    h: usize,
    px: Vec<[f32; 3]>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![[0.15; 3]; w * h],
        }
    }

    fn blend(&mut self, x: i32, y: i32, color: [f32; 4]) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let idx = y as usize * self.w + x as usize;
        let a = color[3].clamp(0.0, 1.0);
        let px = &mut self.px[idx];
        px[0] = color[0] + px[0] * (1.0 - a);
        px[1] = color[1] + px[1] * (1.0 - a);
        px[2] = color[2] + px[2] * (1.0 - a);
    }

    fn tri(&mut self, v: [Pos2; 3], c: [[f32; 4]; 3], clip: Rect) {
        let edge =
            |a: Pos2, b: Pos2, p: Pos2| (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x);
        let area = edge(v[0], v[1], v[2]);
        if area.abs() < 1e-6 {
            return;
        }
        let minx = v
            .iter()
            .map(|p| p.x)
            .fold(f32::MAX, f32::min)
            .max(clip.min.x)
            .floor() as i32;
        let maxx = v
            .iter()
            .map(|p| p.x)
            .fold(f32::MIN, f32::max)
            .min(clip.max.x)
            .ceil() as i32;
        let miny = v
            .iter()
            .map(|p| p.y)
            .fold(f32::MAX, f32::min)
            .max(clip.min.y)
            .floor() as i32;
        let maxy = v
            .iter()
            .map(|p| p.y)
            .fold(f32::MIN, f32::max)
            .min(clip.max.y)
            .ceil() as i32;
        for y in miny..=maxy {
            for x in minx..=maxx {
                let p = pos2(x as f32 + 0.5, y as f32 + 0.5);
                let w0 = edge(v[1], v[2], p) / area;
                let w1 = edge(v[2], v[0], p) / area;
                let w2 = edge(v[0], v[1], p) / area;
                if w0 >= -1e-4 && w1 >= -1e-4 && w2 >= -1e-4 {
                    let mut col = [0.0_f32; 4];
                    for k in 0..4 {
                        col[k] = w0 * c[0][k] + w1 * c[1][k] + w2 * c[2][k];
                    }
                    self.blend(x, y, col);
                }
            }
        }
    }

    fn save_ppm(&self, path: &str) -> std::io::Result<()> {
        let mut f = std::fs::File::create(path)?;
        write!(f, "P6\n{} {}\n255\n", self.w, self.h)?;
        for p in &self.px {
            for c in p {
                f.write_all(&[(c.clamp(0.0, 1.0) * 255.0) as u8])?;
            }
        }
        Ok(())
    }
}

fn color(c: Color32) -> [f32; 4] {
    let a = c.to_array();
    [
        a[0] as f32 / 255.0,
        a[1] as f32 / 255.0,
        a[2] as f32 / 255.0,
        a[3] as f32 / 255.0,
    ]
}

fn main() {
    let kinds: [(u8, &str); 14] = [
        (0, "arrow"),
        (1, "ibeam"),
        (2, "hand"),
        (3, "cross"),
        (4, "wait"),
        (5, "appstarting"),
        (6, "no"),
        (7, "help"),
        (8, "size-ns"),
        (9, "size-we"),
        (10, "size-nwse"),
        (11, "size-nesw"),
        (12, "size-all"),
        (13, "uparrow"),
    ];
    let rows = kinds.len().div_ceil(2);
    let mut canvas = Canvas::new(CELL_W * 2, CELL_H * rows);

    let ctx = Context::default();
    ctx.set_fonts(FontDefinitions::default());

    for (i, (kind, name)) in kinds.iter().enumerate() {
        let x0 = (i % 2) as f32 * CELL_W as f32;
        let y0 = (i / 2) as f32 * CELL_H as f32;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                vec2(canvas.w as f32, canvas.h as f32),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            let painter = ui.painter();
            let third = CELL_W as f32 / 3.0;
            painter.rect_filled(
                Rect::from_min_size(pos2(x0, y0), vec2(third, CELL_H as f32)),
                0.0,
                Color32::WHITE,
            );
            painter.rect_filled(
                Rect::from_min_size(pos2(x0 + third, y0), vec2(third, CELL_H as f32)),
                0.0,
                Color32::BLACK,
            );
            painter.rect_filled(
                Rect::from_min_size(pos2(x0 + 2.0 * third, y0), vec2(third, CELL_H as f32)),
                0.0,
                Color32::from_gray(128),
            );
            let c = pos2(x0 + 90.0, y0 + 100.0);
            paint_cursor(painter, c, SCALE, *kind);
        });
        output.textures_delta.clear();
        for prim in ctx.tessellate(output.shapes, 1.0) {
            if let ClippedPrimitive {
                clip_rect,
                primitive: Primitive::Mesh(mesh),
            } = prim
            {
                let (tris, _) = mesh.indices.as_chunks::<3>();
                for tri in tris {
                    let v = [
                        mesh.vertices[tri[0] as usize],
                        mesh.vertices[tri[1] as usize],
                        mesh.vertices[tri[2] as usize],
                    ];
                    canvas.tri(
                        [v[0].pos, v[1].pos, v[2].pos],
                        [color(v[0].color), color(v[1].color), color(v[2].color)],
                        clip_rect,
                    );
                }
            }
        }
        println!("готово: {name}");
    }

    let out = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/cursor-preview.ppm"
    );
    canvas.save_ppm(out).unwrap();
    println!("сохранено: {out}");
}
