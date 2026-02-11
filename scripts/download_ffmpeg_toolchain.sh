#!/usr/bin/env bash
set -euo pipefail

platform="auto"
repo_root=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --platform)
      platform="${2:-}"
      shift 2
      ;;
    --repo-root)
      repo_root="${2:-}"
      shift 2
      ;;
    *)
      echo "unknown arg: $1" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$repo_root" ]]; then
  repo_root="$(cd "$(dirname "$0")/.." && pwd)"
else
  repo_root="$(cd "$repo_root" && pwd)"
fi

manifest="$repo_root/apps/desktop/src-tauri/toolchain/ffmpeg_manifest.json"
if [[ ! -f "$manifest" ]]; then
  echo "FAIL: manifest missing: $manifest" >&2
  exit 2
fi

for cmd in jq curl sha256sum tar unzip; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "FAIL: missing command: $cmd" >&2
    exit 2
  fi
done

detect_platform() {
  local os arch
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m | tr '[:upper:]' '[:lower:]')"
  case "$os/$arch" in
    linux/x86_64|linux/amd64)
      echo "linux-x86_64"
      ;;
    msys_nt*/x86_64|mingw64_nt*/x86_64|cygwin_nt*/x86_64)
      echo "windows-x86_64"
      ;;
    *)
      echo ""
      ;;
  esac
}

platforms=()
case "$platform" in
  all)
    platforms=("windows-x86_64" "linux-x86_64")
    ;;
  auto)
    p="$(detect_platform)"
    if [[ -z "$p" ]]; then
      echo "FAIL: unsupported host platform for auto mode" >&2
      exit 2
    fi
    platforms=("$p")
    ;;
  windows-x86_64|linux-x86_64)
    platforms=("$platform")
    ;;
  *)
    echo "FAIL: invalid --platform value: $platform" >&2
    exit 2
    ;;
esac

work_root="$(mktemp -d)"
trap 'rm -rf "$work_root"' EXIT

for p in "${platforms[@]}"; do
  echo "INFO: prepare ffmpeg toolchain for $p"

  jq -e ".platforms[\"$p\"]" "$manifest" >/dev/null

  archive_url="$(jq -r ".platforms[\"$p\"].archive_url" "$manifest")"
  archive_sha256="$(jq -r ".platforms[\"$p\"].archive_sha256" "$manifest")"
  archive_type="$(jq -r ".platforms[\"$p\"].archive_type" "$manifest")"
  archive_root="$(jq -r ".platforms[\"$p\"].archive_root" "$manifest")"
  ffmpeg_file="$(jq -r ".platforms[\"$p\"].ffmpeg_file" "$manifest")"
  ffprobe_file="$(jq -r ".platforms[\"$p\"].ffprobe_file" "$manifest")"
  ffmpeg_sha256="$(jq -r ".platforms[\"$p\"].ffmpeg_sha256" "$manifest")"
  ffprobe_sha256="$(jq -r ".platforms[\"$p\"].ffprobe_sha256" "$manifest")"

  work_dir="$work_root/$p"
  mkdir -p "$work_dir"
  archive_path="$work_dir/archive"
  unpack_dir="$work_dir/unpack"
  mkdir -p "$unpack_dir"

  curl -fL "$archive_url" -o "$archive_path"

  got_archive_sha256="$(sha256sum "$archive_path" | awk '{print $1}')"
  if [[ "${got_archive_sha256,,}" != "${archive_sha256,,}" ]]; then
    echo "FAIL: archive sha256 mismatch for $p" >&2
    echo "  expected=$archive_sha256" >&2
    echo "  actual=$got_archive_sha256" >&2
    exit 1
  fi

  case "$archive_type" in
    zip)
      unzip -q "$archive_path" -d "$unpack_dir"
      src_ffmpeg="$unpack_dir/$archive_root/bin/$ffmpeg_file"
      src_ffprobe="$unpack_dir/$archive_root/bin/$ffprobe_file"
      ;;
    tar.xz)
      tar -xf "$archive_path" -C "$unpack_dir"
      src_ffmpeg="$unpack_dir/$archive_root/$ffmpeg_file"
      src_ffprobe="$unpack_dir/$archive_root/$ffprobe_file"
      ;;
    *)
      echo "FAIL: unsupported archive type: $archive_type" >&2
      exit 2
      ;;
  esac

  if [[ ! -f "$src_ffmpeg" || ! -f "$src_ffprobe" ]]; then
    echo "FAIL: expected binaries missing after extraction for $p" >&2
    echo "  ffmpeg=$src_ffmpeg" >&2
    echo "  ffprobe=$src_ffprobe" >&2
    exit 1
  fi

  dest_dir="$repo_root/apps/desktop/src-tauri/toolchain/bin/$p"
  mkdir -p "$dest_dir"
  cp "$src_ffmpeg" "$dest_dir/$ffmpeg_file"
  cp "$src_ffprobe" "$dest_dir/$ffprobe_file"

  if [[ "$p" == "linux-x86_64" ]]; then
    chmod +x "$dest_dir/$ffmpeg_file" "$dest_dir/$ffprobe_file"
  fi

  got_ffmpeg_sha256="$(sha256sum "$dest_dir/$ffmpeg_file" | awk '{print $1}')"
  got_ffprobe_sha256="$(sha256sum "$dest_dir/$ffprobe_file" | awk '{print $1}')"

  if [[ "${got_ffmpeg_sha256,,}" != "${ffmpeg_sha256,,}" ]]; then
    echo "FAIL: ffmpeg sha256 mismatch for $p" >&2
    echo "  expected=$ffmpeg_sha256" >&2
    echo "  actual=$got_ffmpeg_sha256" >&2
    exit 1
  fi
  if [[ "${got_ffprobe_sha256,,}" != "${ffprobe_sha256,,}" ]]; then
    echo "FAIL: ffprobe sha256 mismatch for $p" >&2
    echo "  expected=$ffprobe_sha256" >&2
    echo "  actual=$got_ffprobe_sha256" >&2
    exit 1
  fi

  echo "PASS: $p -> $dest_dir"
done

echo "DONE"
