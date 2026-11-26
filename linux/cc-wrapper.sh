#!/bin/bash
# Обёртка «cc» для сборки Rust-кода relay на этой машине: системного C-тулчейна
# нет (нет gcc/clang, нет crt/libc.so из -dev), поэтому линкуем через rust-lld
# с sysroot, собранным из пакетов libc6-dev + libgcc-12-dev.
# Подготовка sysroot описана в linux/PORTING.md.
#
# Обёртка принимает тот же набор аргументов, что rustc передаёт настоящему cc:
#   -Wl,a,b        → отдельные аргументы линкера
#   -m64/-m32      → игнорируются (целевая архитектура задана)
#   -fuse-ld=…     → игнорируется
#   -B<dir>        → игнорируется (каталог gcc-ld от rustc)
#   -nodefaultlibs → учитываем как «без crt»
set -euo pipefail

SYSROOT="${RAT_SYSROOT:-$HOME/sysroot}"
LIBDIR="$SYSROOT/usr/lib/x86_64-linux-gnu"
GCCDIR="$SYSROOT/usr/lib/gcc/x86_64-linux-gnu/12"
LLD="${RAT_LLD:-$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld}"

args=()
shared=0
nostdlib=0
pie=1
compile_only=0
out=""

i=1
while [ $i -le $# ]; do
  a="${!i}"
  case "$a" in
    -c|-S|-E|-M|-MM) compile_only=1 ;;
    -shared) shared=1; args+=("-shared") ;;
    -nostdlib|-nostartfiles) nostdlib=1 ;;
    -nodefaultlibs) : ;;
    -no-pie) pie=0 ;;
    -pie) pie=1 ;;
    -m64|-m32|-march=*|-mtune=*) : ;;
    -fuse-ld=*) : ;;
    -B*) : ;;
    -o)
      i=$((i+1))
      out="${!i}"
      args+=("-o" "$out")
      i=$((i+1))
      continue
      ;;
    -o*) out="${a#-o}" ;;
    -Wl,*)
      # Разворачиваем -Wl,a,b в -a -b
      IFS=',' read -r -a parts <<< "${a#-Wl,}"
      for p in "${parts[@]}"; do args+=("$p"); done
      ;;
    *) args+=("$a") ;;
  esac
  i=$((i+1))
 done

# rustc собирает proc-macro как обычный dylib и не всегда передаёт -shared явно.
# Определяем по расширению целевого файла: .so/.dylib — разделяемый объект.
case "$out" in
  *.so|*.so.*|*.dylib) shared=1 ;;
esac

# Для shared-объекта PIE-флаги неуместны.
if [ "$shared" = "1" ]; then
  pie=0
  filtered=()
  for a in "${args[@]}"; do
    case "$a" in -pie|-fPIE|-fpie) : ;; *) filtered+=("$a") ;; esac
  done
  args=("${filtered[@]}")
fi

if [ "$compile_only" = "1" ]; then
  echo "cc-wrapper: компиляция C без линковки не поддерживается (в sysroot нет cc1)" >&2
  exit 1
fi

crt=()
tail_crt=()
if [ "$nostdlib" = "0" ]; then
  if [ "$shared" = "1" ]; then
    crt=("$LIBDIR/crti.o" "$GCCDIR/crtbeginS.o")
    tail_crt=("$GCCDIR/crtendS.o" "$LIBDIR/crtn.o")
  elif [ "$pie" = "1" ]; then
    crt=("$LIBDIR/Scrt1.o" "$LIBDIR/crti.o" "$GCCDIR/crtbeginS.o")
    tail_crt=("$GCCDIR/crtendS.o" "$LIBDIR/crtn.o")
  else
    crt=("$LIBDIR/crt1.o" "$LIBDIR/crti.o" "$GCCDIR/crtbegin.o")
    tail_crt=("$GCCDIR/crtend.o" "$LIBDIR/crtn.o")
  fi
fi

exec "$LLD" -flavor gnu \
  --sysroot="$SYSROOT" \
  -L"$LIBDIR" \
  -L"$GCCDIR" \
  -L/lib/x86_64-linux-gnu \
  -L/usr/lib/x86_64-linux-gnu \
  -dynamic-linker /lib64/ld-linux-x86-64.so.2 \
  "${crt[@]}" "${args[@]}" "${tail_crt[@]}"
