# rs_music_server

A self-hosted music streaming server written in Rust. Serves a randomised weighted song queue via a web UI, supports file uploads, and provides a YouTube download pipeline that imports tracks directly into the library.

## Architecture

- **Web layer**: Actix-web serving both a browser UI (MiniJinja templates) and a JSON API.
- **Database**: SQLite via `rusqlite` (bundled). Schema migrations are version-gated in `src/update_manager.rs`.
- **Download worker**: Background Tokio task supervisor in `src/dl.rs` that drives a per-job state machine through download, normalisation, ffmpeg processing, rename, and DB import steps.
- **Song selection**: Weighted random queue with replay protection and adjustable rating scale.

## Runtime Dependencies

| Tool | Purpose |
|------|---------|
| `yt-dlp` | YouTube audio download |
| `mp3gain` | Replay-gain normalisation |
| `ffmpeg` | Compand/loudness adjustment |

## Quick Start

```sh
# environment variables (all optional, defaults shown)
export PORT=3000
export PORT_INTERNAL=3001
export MUSICDIR=./music
export DBDIR=.
export UPLOADDIR=./music/upload

cargo run --release
```

## Features

### YouTube download job queue (2026-03-15)
Adds a persistent `download_jobs` table (DB schema v5) and a full multi-step download pipeline driven by a background Tokio supervisor.
Users enqueue a YouTube URL with song metadata from `/web/jobs`; the worker drives each job through download → ensure_mp3 → mp3gain → ffmpeg → rename → move → DB import, persisting step state and file paths for idempotent restart recovery.
The download step now requests `bestaudio/best` and `ensure_mp3` converts non-MP3 downloads to MP3 via `ffmpeg` before normalization.
The jobs page polls `/jobs/last-completed-at` and shows a refresh prompt when new completions arrive; per-step error output is stored and displayed inline.
Per-job cookie files (`/cookies/job-<id>.txt`) support yt-dlp authentication and are deleted after the download step.
