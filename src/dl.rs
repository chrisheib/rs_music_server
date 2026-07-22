use actix_web::{
    get, post,
    web::{self, Data, Json},
    HttpResponse,
};
use minijinja::context;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::sleep;

use crate::{db::db_con, update_manager::db_update, AppState, MyRes};

const INSERT_JOB_SONG_STMT: &str = "INSERT INTO songs (path, filename, songname, artist, album, length, seconds, rating, vote, deleted)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT (path) DO UPDATE SET
        songname=excluded.songname,
        artist=excluded.artist,
        album=excluded.album,
        length=excluded.length,
        seconds=excluded.seconds,
        rating=excluded.rating,
        deleted=excluded.deleted";
const JOB_ERROR_MAX_LEN: usize = 1000;
const JOB_POLL_INTERVAL_SECS: u64 = 3;
const JOB_COOKIE_MAX_LEN: usize = 128 * 1024;
const FFMPEG_COMPAND_FILTER: &str = "compand=attacks=0.1:decays=0.1:soft-knee=5:points=-120/-120|-80/-80|-60/-40|-40/-20|-20/-15|0/-1|10/-1";
const YTDLP_FORMAT_SELECTORS: [Option<&str>; 3] = [Some("bestaudio/best"), Some("best"), None];
const YTDLP_YOUTUBE_EXTRACTOR_ARGS: &str = "youtube:player_client=default,-tv,-tv_downgraded";
const YTDLP_YOUTUBE_EXTRACTOR_ARGS_WEB_ANDROID: &str =
    "youtube:player_client=web,android,-tv,-tv_downgraded";
const YTDLP_POLITE_SLEEP_REQUESTS: &str = "1.0";
const YTDLP_POLITE_SLEEP_INTERVAL: &str = "1";
const YTDLP_POLITE_MAX_SLEEP_INTERVAL: &str = "3";
const YTDLP_POLITE_LIMIT_RATE: &str = "1M";
const YTDLP_LIST_FORMATS_LOG_MAX_CHARS: usize = 3000;

/// Website with joblist
#[get("/web/jobs")]
pub async fn net_jobs_web(app: Data<AppState>) -> MyRes<HttpResponse> {
    log_job_info("rendering jobs page");
    db_update()?;
    let jobs = read_jobs_db()?;
    let rendered = app.render_template(
        "jobs.html",
        context! {jobs => &jobs, build_timestamp => app.build_timestamp()},
    )?;
    Ok(HttpResponse::Ok().body(rendered))
}

#[derive(Debug, Serialize)]
pub struct Job {
    pub id: i32,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub url: String,
    pub songname: String,
    pub artist: String,
    pub album: String,
    pub rating: i32,
    pub status: JobStatus,
    pub step: JobStep,
    pub step_index: i32,
    pub attempt_count: i32,
    pub song_id: Option<i32>,
    pub error_message: String,
    pub temp_dir: String,
    pub downloaded_path: String,
    pub normalized_path: String,
    pub final_path: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

impl JobStatus {
    /// Converts an enum value into the canonical database string representation.
    pub fn as_db_str(self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Completed => "completed",
            JobStatus::Failed => "failed",
        }
    }

    /// Parses a database status string into the strongly typed enum.
    pub fn from_db_str(raw: &str) -> Option<Self> {
        match raw {
            "queued" => Some(JobStatus::Queued),
            "running" => Some(JobStatus::Running),
            "completed" => Some(JobStatus::Completed),
            "failed" => Some(JobStatus::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum JobStep {
    Created,
    Downloading,
    EnsureMp3,
    Mp3Gain,
    FfmpegAdjust,
    Rename,
    MoveToMusicDir,
    ImportDb,
    Done,
}

impl JobStep {
    /// Converts an enum value into the canonical database string representation.
    pub fn as_db_str(self) -> &'static str {
        match self {
            JobStep::Created => "created",
            JobStep::Downloading => "downloading",
            JobStep::EnsureMp3 => "ensure_mp3",
            JobStep::Mp3Gain => "mp3gain",
            JobStep::FfmpegAdjust => "ffmpeg_adjust",
            JobStep::Rename => "rename",
            JobStep::MoveToMusicDir => "move_to_music_dir",
            JobStep::ImportDb => "import_db",
            JobStep::Done => "done",
        }
    }

    /// Parses a database step string into the strongly typed enum.
    pub fn from_db_str(raw: &str) -> Option<Self> {
        match raw {
            "created" => Some(JobStep::Created),
            "downloading" => Some(JobStep::Downloading),
            "ensure_mp3" => Some(JobStep::EnsureMp3),
            "mp3gain" => Some(JobStep::Mp3Gain),
            "ffmpeg_adjust" => Some(JobStep::FfmpegAdjust),
            "rename" => Some(JobStep::Rename),
            "move_to_music_dir" => Some(JobStep::MoveToMusicDir),
            "import_db" => Some(JobStep::ImportDb),
            "done" => Some(JobStep::Done),
            _ => None,
        }
    }

    /// Returns the stable step order index used for progress tracking in persistence.
    pub fn index(self) -> i32 {
        match self {
            JobStep::Created => 0,
            JobStep::Downloading => 1,
            JobStep::EnsureMp3 => 2,
            JobStep::Mp3Gain => 3,
            JobStep::FfmpegAdjust => 4,
            JobStep::Rename => 5,
            JobStep::MoveToMusicDir => 6,
            JobStep::ImportDb => 7,
            JobStep::Done => 8,
        }
    }
}

/// Payload used by the queue creation endpoint to enqueue new work items.
#[derive(Debug, Deserialize)]
pub struct CreateJobRequest {
    pub url: String,
    pub songname: String,
    pub artist: String,
    pub album: String,
    pub rating: i32,
    pub cookies: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateJobResponse {
    pub id: i32,
    pub status: JobStatus,
    pub step: JobStep,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct LastCompletedAtResponse {
    pub completed_at: Option<String>,
}

/// Registers all download job page and JSON API routes for the Actix app.
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(net_jobs_web)
        .service(net_jobs)
        .service(net_jobs_create)
        .service(net_jobs_last_completed_at);
}

/// Converts DB row strings into a typed status used across the queue subsystem.
fn parse_job_status(raw: &str) -> JobStatus {
    JobStatus::from_db_str(raw).unwrap_or(JobStatus::Failed)
}

/// Converts DB row strings into a typed step used across the queue subsystem.
fn parse_job_step(raw: &str) -> JobStep {
    JobStep::from_db_str(raw).unwrap_or(JobStep::Created)
}

/// Maps one row from `download_jobs` into the strongly typed `Job` domain model.
fn row_to_job(row: &rusqlite::Row<'_>) -> std::result::Result<Job, rusqlite::Error> {
    let status_raw = row.get::<_, String>(9)?;
    let step_raw = row.get::<_, String>(10)?;

    Ok(Job {
        id: row.get::<_, i32>(0)?,
        created_at: row.get::<_, String>(1)?,
        updated_at: row.get::<_, String>(2)?,
        completed_at: row.get::<_, Option<String>>(3)?,
        url: row.get::<_, String>(4)?,
        songname: row.get::<_, String>(5)?,
        artist: row.get::<_, String>(6)?,
        album: row.get::<_, String>(7)?,
        rating: row.get::<_, i32>(8)?,
        status: parse_job_status(&status_raw),
        step: parse_job_step(&step_raw),
        step_index: row.get::<_, i32>(11)?,
        attempt_count: row.get::<_, i32>(12)?,
        song_id: row.get::<_, Option<i32>>(13)?,
        error_message: row.get::<_, String>(14)?,
        temp_dir: row.get::<_, String>(15)?,
        downloaded_path: row.get::<_, String>(16)?,
        normalized_path: row.get::<_, String>(17)?,
        final_path: row.get::<_, String>(18)?,
    })
}

/// Inserts a new queue item and returns the generated job id.
pub fn create_job_db(data: &CreateJobRequest) -> MyRes<i64> {
    let conn = db_con()?;
    let sql = "INSERT INTO download_jobs (
        created_at,
        updated_at,
        completed_at,
        url,
        songname,
        artist,
        album,
        rating,
        status,
        step,
        step_index,
        attempt_count,
        song_id,
        error_message,
        temp_dir,
        downloaded_path,
        normalized_path,
        final_path
    ) VALUES (
        datetime('now'),
        datetime('now'),
        NULL,
        ?, ?, ?, ?, ?, ?, ?, ?, 0, NULL, '', '', '', '', ''
    )";

    conn.execute(
        sql,
        (
            data.url.trim(),
            data.songname.trim(),
            data.artist.trim(),
            data.album.trim(),
            data.rating,
            JobStatus::Queued.as_db_str(),
            JobStep::Created.as_db_str(),
            JobStep::Created.index(),
        ),
    )?;

    Ok(conn.last_insert_rowid())
}

/// Reads all queue items ordered by newest-first for admin and UI displays.
pub fn read_jobs_db() -> MyRes<Vec<Job>> {
    let conn = db_con()?;
    let sql = "SELECT
        id,
        created_at,
        updated_at,
        completed_at,
        url,
        songname,
        artist,
        album,
        rating,
        status,
        step,
        step_index,
        attempt_count,
        song_id,
        error_message,
        temp_dir,
        downloaded_path,
        normalized_path,
        final_path
    FROM download_jobs
    ORDER BY id DESC";

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], row_to_job)?;

    let mut jobs = Vec::new();
    for row in rows {
        jobs.push(row?);
    }

    Ok(jobs)
}

/// Reads one queue item by id and returns `None` when no row exists.
pub fn read_job_db(job_id: i32) -> MyRes<Option<Job>> {
    let conn = db_con()?;
    let sql = "SELECT
        id,
        created_at,
        updated_at,
        completed_at,
        url,
        songname,
        artist,
        album,
        rating,
        status,
        step,
        step_index,
        attempt_count,
        song_id,
        error_message,
        temp_dir,
        downloaded_path,
        normalized_path,
        final_path
    FROM download_jobs
    WHERE id = ?";

    let mut stmt = conn.prepare(sql)?;
    let row = stmt.query_row([job_id], row_to_job).optional()?;

    Ok(row)
}

/// Updates queue progress status and step and refreshes `updated_at`.
pub fn update_job_stage(
    job_id: i32,
    status: JobStatus,
    step: JobStep,
    attempt_count: i32,
    error_message: &str,
) -> MyRes<bool> {
    let conn = db_con()?;
    let changed = conn.execute(
        "UPDATE download_jobs
         SET updated_at = datetime('now'),
             status = ?,
             step = ?,
             step_index = ?,
             attempt_count = ?,
             error_message = ?
         WHERE id = ?",
        (
            status.as_db_str(),
            step.as_db_str(),
            step.index(),
            attempt_count,
            error_message,
            job_id,
        ),
    )?;

    Ok(changed > 0)
}

/// Persists file path artifacts produced by worker steps.
pub fn update_job_paths(
    job_id: i32,
    temp_dir: &str,
    downloaded_path: &str,
    normalized_path: &str,
    final_path: &str,
) -> MyRes<bool> {
    let conn = db_con()?;
    let changed = conn.execute(
        "UPDATE download_jobs
         SET updated_at = datetime('now'),
             temp_dir = ?,
             downloaded_path = ?,
             normalized_path = ?,
             final_path = ?
         WHERE id = ?",
        (
            temp_dir,
            downloaded_path,
            normalized_path,
            final_path,
            job_id,
        ),
    )?;

    Ok(changed > 0)
}

/// Marks a queue item as completed, stores `song_id`, and stamps completion time.
pub fn mark_job_completed(job_id: i32, song_id: i32) -> MyRes<bool> {
    let conn = db_con()?;
    let changed = conn.execute(
        "UPDATE download_jobs
         SET updated_at = datetime('now'),
             completed_at = datetime('now'),
             status = ?,
             step = ?,
             step_index = ?,
             song_id = ?,
             error_message = ''
         WHERE id = ?",
        (
            JobStatus::Completed.as_db_str(),
            JobStep::Done.as_db_str(),
            JobStep::Done.index(),
            song_id,
            job_id,
        ),
    )?;

    Ok(changed > 0)
}

/// Marks a queue item as failed and stores failure details with completion time.
pub fn mark_job_failed(job_id: i32, step: JobStep, error_message: &str) -> MyRes<bool> {
    let conn = db_con()?;
    let changed = conn.execute(
        "UPDATE download_jobs
         SET updated_at = datetime('now'),
             completed_at = datetime('now'),
             status = ?,
             step = ?,
             step_index = ?,
             error_message = ?
         WHERE id = ?",
        (
            JobStatus::Failed.as_db_str(),
            step.as_db_str(),
            step.index(),
            error_message,
            job_id,
        ),
    )?;

    Ok(changed > 0)
}

/// Deletes a queue item, primarily for manual cleanup flows.
pub fn delete_job_db(job_id: i32) -> MyRes<bool> {
    let conn = db_con()?;
    let changed = conn.execute("DELETE FROM download_jobs WHERE id = ?", [job_id])?;
    Ok(changed > 0)
}

/// Returns the newest non-null completion timestamp for client-side refresh checks.
pub fn read_last_completed_at_db() -> MyRes<Option<String>> {
    let conn = db_con()?;
    let mut stmt = conn.prepare("SELECT MAX(completed_at) FROM download_jobs")?;
    Ok(stmt.query_row([], |row| row.get::<_, Option<String>>(0))?)
}

/// Returns all queue rows as JSON for the jobs frontend refresh flow.
#[get("/jobs")]
pub async fn net_jobs() -> MyRes<Json<Vec<Job>>> {
    db_update()?;
    let jobs = read_jobs_db()?;
    Ok(Json(jobs))
}

/// Returns the latest completion timestamp used by polling-based refresh checks.
#[get("/jobs/last-completed-at")]
pub async fn net_jobs_last_completed_at() -> MyRes<Json<LastCompletedAtResponse>> {
    db_update()?;
    let completed_at = read_last_completed_at_db()?;
    Ok(Json(LastCompletedAtResponse { completed_at }))
}

/// API endpoint to enqueue a new download job.
#[post("/jobs")]
pub async fn net_jobs_create(payload: Json<CreateJobRequest>) -> MyRes<Json<CreateJobResponse>> {
    db_update()?;

    let mut request = payload.into_inner();
    let cookie_payload = extract_cookie_payload(request.cookies.as_deref());

    if request.url.trim().is_empty() {
        return Err("Please provide a YouTube URL.".into());
    }

    if !(0..=7).contains(&request.rating) {
        return Err("Rating must be between 0 and 7.".into());
    }

    if let Some(cookie_text) = cookie_payload.as_deref() {
        validate_cookie_payload_len(cookie_text)?;
    }

    request.url = sanitize_job_url(&request.url);

    let inserted_id = create_job_db(&request)?;
    let inserted_id_i32 = i32::try_from(inserted_id)?;

    if let Some(cookie_text) = cookie_payload.as_deref() {
        if let Err(err) = write_job_cookie_file(inserted_id_i32, cookie_text) {
            let _ = delete_job_db(inserted_id_i32);
            return Err(
                format!("Failed to persist cookie file for job {inserted_id_i32}: {err}").into(),
            );
        }
    }

    let created_job = read_job_db(inserted_id_i32)?;

    let row = match created_job {
        Some(job) => job,
        None => return Err("created job row missing".into()),
    };

    Ok(Json(CreateJobResponse {
        id: row.id,
        status: row.status,
        step: row.step,
        created_at: row.created_at,
    }))
}

/// Imports a successfully processed job file into the songs table and stores `song_id`.
pub fn import_job_song(job_id: i32) -> MyRes<i32> {
    db_update()?;

    let job = match read_job_db(job_id)? {
        Some(found) => found,
        None => return Err(format!("job {job_id} not found").into()),
    };

    let final_path = job.final_path.trim();
    if final_path.is_empty() {
        return Err(format!("job {job_id} has no final_path to import").into());
    }

    if !Path::new(final_path).exists() {
        return Err(format!("job {job_id} final_path does not exist: {final_path}").into());
    }

    let filename = Path::new(final_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();

    let songname = sanitize_song_field(&job.songname, &filename);
    let artist = sanitize_song_field(&job.artist, "");
    let album = sanitize_song_field(&job.album, "");
    let rating = clamp_rating(job.rating);
    let seconds = get_songlength_secs(final_path);
    let length = format_songlength(seconds);

    let conn = db_con()?;
    conn.execute(
        INSERT_JOB_SONG_STMT,
        (
            final_path, filename, songname, artist, album, length, seconds, rating, 0, 0,
        ),
    )?;

    let song_id = conn.query_row("SELECT id FROM songs WHERE path = ?", [final_path], |row| {
        row.get::<_, i32>(0)
    })?;

    delete_job_upload_dir_if_exists(job_id)?;
    mark_job_completed(job_id, song_id)?;
    Ok(song_id)
}

/// Normalizes nullable metadata fields by trimming and applying a fallback when needed.
fn sanitize_song_field(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        return trimmed.to_owned();
    }

    fallback.trim().to_owned()
}

/// Normalizes incoming download URLs and strips non-essential YouTube tracking/list params.
fn sanitize_job_url(raw_url: &str) -> String {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Some(video_id) = extract_youtube_video_id(trimmed) {
        return format!("https://www.youtube.com/watch?v={video_id}");
    }

    trimmed.to_string()
}

/// Extracts a canonical YouTube video id from common watch/share URL variants.
fn extract_youtube_video_id(url: &str) -> Option<String> {
    if let Some(candidate) = query_param_value(url, "v") {
        return normalize_youtube_video_id(candidate);
    }

    if let Some(path) = strip_prefix_case_insensitive(url, "https://youtu.be/")
        .or_else(|| strip_prefix_case_insensitive(url, "http://youtu.be/"))
    {
        let segment = path.split(['?', '&', '#', '/']).next().unwrap_or_default();
        return normalize_youtube_video_id(segment);
    }

    if let Some(path) = strip_prefix_case_insensitive(url, "https://www.youtube.com/shorts/")
        .or_else(|| strip_prefix_case_insensitive(url, "http://www.youtube.com/shorts/"))
        .or_else(|| strip_prefix_case_insensitive(url, "https://youtube.com/shorts/"))
        .or_else(|| strip_prefix_case_insensitive(url, "http://youtube.com/shorts/"))
    {
        let segment = path.split(['?', '&', '#', '/']).next().unwrap_or_default();
        return normalize_youtube_video_id(segment);
    }

    None
}

/// Returns query parameter value for a key when present in the URL.
fn query_param_value<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let query = url.split('?').nth(1)?;
    let query = query.split('#').next().unwrap_or_default();

    for part in query.split('&') {
        let mut pieces = part.splitn(2, '=');
        let current_key = pieces.next().unwrap_or_default();
        let value = pieces.next().unwrap_or_default();
        if current_key == key {
            return Some(value);
        }
    }

    None
}

/// Trims URL prefixes in a case-insensitive way and returns the remaining suffix.
fn strip_prefix_case_insensitive<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    if value.len() < prefix.len() {
        return None;
    }

    if value[..prefix.len()].eq_ignore_ascii_case(prefix) {
        return Some(&value[prefix.len()..]);
    }

    None
}

/// Validates and cleans a probable YouTube video id.
fn normalize_youtube_video_id(candidate: &str) -> Option<String> {
    let sanitized: String = candidate
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();

    if sanitized.is_empty() {
        return None;
    }

    Some(sanitized)
}

/// Clamps queue-provided rating values into the application's supported rating range.
fn clamp_rating(rating: i32) -> i32 {
    rating.clamp(0, 7)
}

/// Reads mp3 duration in seconds and returns `0` when metadata parsing fails.
fn get_songlength_secs(path: &str) -> u64 {
    let parsed_path = Path::new(path);
    let duration = mp3_duration::from_path(parsed_path).unwrap_or_default();
    duration.as_secs()
}

/// Formats duration seconds to `M:SS` or `H:MM:SS` for songs table compatibility.
fn format_songlength(seconds: u64) -> String {
    let mins = seconds / 60;
    let secs = seconds % 60;
    if mins >= 60 {
        let hours = mins / 60;
        let remaining_mins = mins % 60;
        return format!("{hours}:{remaining_mins:0>2}:{secs:0>2}");
    }

    format!("{mins:0>1}:{secs:0>2}")
}

/// Starts the background supervisor for queued download jobs.
pub fn start_job_worker() -> JoinHandle<()> {
    tokio::spawn(async {
        if let Err(err) = job_supervisor_loop().await {
            log_job_error(&format!("supervisor stopped unexpectedly: {err}"));
        }
    })
}

/// Runs startup recovery and continuously claims and processes queued jobs.
async fn job_supervisor_loop() -> MyRes<()> {
    db_update()?;

    sweep_stale_job_cookie_files()?;
    log_tool_availability_warnings();

    let recovered = reset_running_jobs_for_recovery()?;
    if recovered > 0 {
        log_job_info(&format!(
            "recovered {recovered} running jobs back to queued state"
        ));
    }

    loop {
        let claimed = claim_next_queued_job()?;
        let Some(job) = claimed else {
            sleep(Duration::from_secs(JOB_POLL_INTERVAL_SECS)).await;
            continue;
        };

        if let Err(err) = process_claimed_job(&job) {
            let message =
                truncate_error_message(&format!("State machine failed for job {}: {err}", job.id));
            log_job_error(&message);
        }
    }
}

/// Re-queues jobs that were left in `running` state after an unclean shutdown.
fn reset_running_jobs_for_recovery() -> MyRes<usize> {
    let conn = db_con()?;
    let changed = conn.execute(
        "UPDATE download_jobs
         SET updated_at = datetime('now'),
             status = ?,
             step = ?,
             step_index = ?,
             attempt_count = attempt_count + 1,
             completed_at = NULL,
             error_message = CASE
                 WHEN TRIM(error_message) = '' THEN 'Recovered after restart; queued again.'
                 ELSE error_message || '\nRecovered after restart; queued again.'
             END
         WHERE status = ?",
        (
            JobStatus::Queued.as_db_str(),
            JobStep::Created.as_db_str(),
            JobStep::Created.index(),
            JobStatus::Running.as_db_str(),
        ),
    )?;

    Ok(changed)
}

/// Claims the oldest queued job by atomically transitioning it to `running`.
fn claim_next_queued_job() -> MyRes<Option<Job>> {
    loop {
        let conn = db_con()?;
        let mut stmt = conn.prepare(
            "SELECT id
             FROM download_jobs
             WHERE status = ?
             ORDER BY created_at ASC, id ASC
             LIMIT 1",
        )?;

        let next_id = stmt
            .query_row([JobStatus::Queued.as_db_str()], |row| row.get::<_, i32>(0))
            .optional()?;

        let Some(job_id) = next_id else {
            return Ok(None);
        };

        let changed = conn.execute(
            "UPDATE download_jobs
             SET updated_at = datetime('now'),
                 status = ?,
                 attempt_count = attempt_count + 1,
                 completed_at = NULL,
                 error_message = ''
             WHERE id = ? AND status = ?",
            (
                JobStatus::Running.as_db_str(),
                job_id,
                JobStatus::Queued.as_db_str(),
            ),
        )?;

        if changed == 0 {
            continue;
        }

        let claimed_job = read_job_db(job_id)?;
        if let Some(job) = claimed_job {
            log_job_info(&format!("claimed job {}", job.id));
            return Ok(Some(job));
        }

        return Err(format!("claimed job {job_id} but row could not be reloaded").into());
    }
}

/// Processes one claimed job and writes completion/failure details to persistence.
fn process_claimed_job(job: &Job) -> MyRes<()> {
    let attempt_count = job.attempt_count;
    let temp_dir = ensure_temp_dir_for_job(job.id)?;
    let mut downloaded_path = job.downloaded_path.clone();
    let mut normalized_path = job.normalized_path.clone();
    let mut final_path = job.final_path.clone();

    update_job_paths(
        job.id,
        temp_dir.to_str().unwrap_or_default(),
        &downloaded_path,
        &normalized_path,
        &final_path,
    )?;

    if execute_or_fail_step(job.id, JobStep::Downloading, || {
        run_downloading_step(job, attempt_count, &temp_dir, &mut downloaded_path)
    })?
    .is_none()
    {
        return Ok(());
    }

    if execute_or_fail_step(job.id, JobStep::EnsureMp3, || {
        run_ensure_mp3_step(job.id, attempt_count, &temp_dir, &mut downloaded_path)
    })?
    .is_none()
    {
        return Ok(());
    }

    if execute_or_fail_step(job.id, JobStep::Mp3Gain, || {
        run_mp3gain_step(
            job.id,
            attempt_count,
            &downloaded_path,
            &mut normalized_path,
        )
    })?
    .is_none()
    {
        return Ok(());
    }

    let Some(adjusted_path) = execute_or_fail_step(job.id, JobStep::FfmpegAdjust, || {
        run_ffmpeg_adjust_step(job.id, attempt_count, &temp_dir, &normalized_path)
    })?
    else {
        return Ok(());
    };

    let Some(renamed_path) = execute_or_fail_step(job.id, JobStep::Rename, || {
        run_rename_step(job, attempt_count, &temp_dir, &adjusted_path)
    })?
    else {
        return Ok(());
    };

    if execute_or_fail_step(job.id, JobStep::MoveToMusicDir, || {
        run_move_step(
            job,
            attempt_count,
            &renamed_path,
            &downloaded_path,
            &normalized_path,
            &mut final_path,
        )
    })?
    .is_none()
    {
        return Ok(());
    }

    let Some(song_id) = execute_or_fail_step(job.id, JobStep::ImportDb, || {
        run_import_step(job.id, attempt_count)
    })?
    else {
        return Ok(());
    };

    log_job_info(&format!(
        "job {} completed with song_id {}",
        job.id, song_id
    ));

    Ok(())
}

/// Runs one job state-machine step and marks the job failed when the step returns an error.
fn execute_or_fail_step<T, F>(job_id: i32, step: JobStep, action: F) -> MyRes<Option<T>>
where
    F: FnOnce() -> MyRes<T>,
{
    match action() {
        Ok(value) => Ok(Some(value)),
        Err(err) => {
            fail_job_with_step(job_id, step, err)?;
            Ok(None)
        }
    }
}

/// Persists step transition to running and emits standardized start logging.
fn start_step(job_id: i32, attempt_count: i32, step: JobStep) -> MyRes<()> {
    update_job_stage(job_id, JobStatus::Running, step, attempt_count, "")?;
    log_job_info(&format!("job {job_id} step {}: start", step.as_db_str()));
    Ok(())
}

/// Emits standardized step completion logging.
fn finish_step(job_id: i32, step: JobStep) {
    log_job_info(&format!("job {job_id} step {}: done", step.as_db_str()));
}

/// Emits standardized step completion logging with additional context.
fn finish_step_with_note(job_id: i32, step: JobStep, note: &str) {
    log_job_info(&format!(
        "job {job_id} step {}: done ({note})",
        step.as_db_str()
    ));
}

/// Removes stale per-job cookie files from previous runs before processing new jobs.
fn sweep_stale_job_cookie_files() -> MyRes<()> {
    let cookie_dir = get_cookie_dir();
    if !cookie_dir.exists() {
        return Ok(());
    }

    let entries = fs::read_dir(&cookie_dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        if !is_job_cookie_filename(filename) {
            continue;
        }

        fs::remove_file(&path)?;
    }

    Ok(())
}

/// Returns true when a filename follows the `job-<id>.txt` cookie naming scheme.
fn is_job_cookie_filename(filename: &str) -> bool {
    let Some(stem) = filename.strip_suffix(".txt") else {
        return false;
    };

    let Some(id_part) = stem.strip_prefix("job-") else {
        return false;
    };

    if id_part.is_empty() {
        return false;
    }

    id_part.chars().all(|character| character.is_ascii_digit())
}

/// Runs the download step and persists the output path.
fn run_downloading_step(
    job: &Job,
    attempt_count: i32,
    temp_dir: &Path,
    downloaded_path: &mut String,
) -> MyRes<()> {
    start_step(job.id, attempt_count, JobStep::Downloading)?;

    if !downloaded_path.trim().is_empty() && Path::new(downloaded_path).exists() {
        delete_job_cookie_file_if_exists(job.id)?;
        finish_step_with_note(job.id, JobStep::Downloading, "skipped, already downloaded");
        return Ok(());
    }

    run_ytdlp_download_for_job(job, temp_dir)?;

    let resolved = find_downloaded_file_in_temp_dir(temp_dir)?;

    *downloaded_path = resolved.to_string_lossy().to_string();
    update_job_paths(
        job.id,
        temp_dir.to_str().unwrap_or_default(),
        downloaded_path,
        "",
        "",
    )?;
    finish_step(job.id, JobStep::Downloading);
    Ok(())
}

/// Runs yt-dlp for one job, optionally using a per-job cookie file and always cleaning it up.
fn run_ytdlp_download_for_job(job: &Job, temp_dir: &Path) -> MyRes<()> {
    let template_path = temp_dir.join("downloaded.%(ext)s");
    let cookie_path = cookie_file_path_for_job(job.id);
    let output_template = template_path.to_string_lossy().to_string();
    let cookie_arg = cookie_path.exists().then_some(cookie_path.as_path());
    let command_error = match run_ytdlp_with_selector_fallback(
        job.id,
        job.url.trim(),
        &output_template,
        None,
        true,
        "guest-polite",
    ) {
        Ok(()) => None,
        Err(guest_error) => {
            if let Some(cookie_file) = cookie_arg {
                log_job_info(&format!(
                    "job {} guest-polite download failed, retrying in cookie mode",
                    job.id
                ));

                match run_ytdlp_with_selector_fallback(
                    job.id,
                    job.url.trim(),
                    &output_template,
                    Some(cookie_file),
                    false,
                    "cookie",
                ) {
                    Ok(()) => None,
                    Err(cookie_error) => Some(format!(
                        "guest-polite failed: {} | cookie fallback failed: {}",
                        guest_error, cookie_error
                    )),
                }
            } else {
                Some(guest_error.to_string())
            }
        }
    };

    let cleanup_result = delete_job_cookie_file_if_exists(job.id);

    if let Some(command_err) = command_error {
        let mut message = command_err;
        if let Err(cleanup_err) = cleanup_result {
            message = format!("{message} | cookie cleanup failed: {cleanup_err}");
        }
        return Err(message.into());
    }

    if let Err(cleanup_err) = cleanup_result {
        return Err(format!("download succeeded but cookie cleanup failed: {cleanup_err}").into());
    }

    Ok(())
}

/// Runs one yt-dlp mode with selector fallbacks and optional dynamic format-id probing.
fn run_ytdlp_with_selector_fallback(
    job_id: i32,
    url: &str,
    output_template: &str,
    cookie_path: Option<&Path>,
    polite_mode: bool,
    mode_name: &str,
) -> MyRes<()> {
    let extractor_profiles = [
        ("none", None),
        ("default", Some(YTDLP_YOUTUBE_EXTRACTOR_ARGS)),
        (
            "web-android",
            Some(YTDLP_YOUTUBE_EXTRACTOR_ARGS_WEB_ANDROID),
        ),
    ];

    let mut last_error_message = String::new();

    for (profile_name, extractor_args) in extractor_profiles {
        log_ytdlp_list_formats_once(
            job_id,
            url,
            cookie_path,
            polite_mode,
            mode_name,
            profile_name,
            extractor_args,
        );

        let mut command_error: Option<String> = None;
        let mut saw_requested_format_unavailable = false;

        for format_selector in YTDLP_FORMAT_SELECTORS {
            let args = build_ytdlp_download_args_with_mode(
                url,
                output_template,
                cookie_path,
                format_selector,
                polite_mode,
                extractor_args,
            );
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

            match run_command_capture_detailed("yt-dlp", &arg_refs) {
                Ok(()) => {
                    return Ok(());
                }
                Err(error) => {
                    let requested_format_unavailable = error.is_requested_format_unavailable();
                    let should_retry_selector_fallback =
                        format_selector.is_some() && requested_format_unavailable;
                    let error_text = error.to_string();

                    if requested_format_unavailable {
                        saw_requested_format_unavailable = true;
                    }

                    if should_retry_selector_fallback {
                        let selector = format_selector.unwrap_or("default");
                        log_job_info(&format!(
                            "job {} yt-dlp mode '{}' profile '{}' selector '{}' unavailable, retrying broader fallback",
                            job_id, mode_name, profile_name, selector
                        ));
                        command_error = Some(error_text);
                        continue;
                    }

                    command_error = Some(error_text);
                    break;
                }
            }
        }

        if command_error.is_some() && saw_requested_format_unavailable {
            match find_best_available_ytdlp_format_id(url, cookie_path, extractor_args) {
                Ok(Some(format_id)) => {
                    let args = build_ytdlp_download_args_with_mode(
                        url,
                        output_template,
                        cookie_path,
                        Some(format_id.as_str()),
                        polite_mode,
                        extractor_args,
                    );
                    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
                    log_job_info(&format!(
                        "job {} mode '{}' profile '{}' retrying yt-dlp with discovered format id '{}'",
                        job_id, mode_name, profile_name, format_id
                    ));

                    match run_command_capture_detailed("yt-dlp", &arg_refs) {
                        Ok(()) => {
                            return Ok(());
                        }
                        Err(error) => {
                            command_error = Some(error.to_string());
                        }
                    }
                }
                Ok(None) => {
                    log_job_info(&format!(
                        "job {} mode '{}' profile '{}' format probe found no viable id",
                        job_id, mode_name, profile_name
                    ));
                }
                Err(error) => {
                    log_job_info(&format!(
                        "job {} mode '{}' profile '{}' format probe failed: {}",
                        job_id, mode_name, profile_name, error
                    ));
                }
            }
        }

        if let Some(message) = command_error {
            last_error_message = message;
            log_job_info(&format!(
                "job {} mode '{}' profile '{}' exhausted, trying next extractor profile",
                job_id, mode_name, profile_name
            ));
        }
    }

    Err(last_error_message.into())
}

/// Executes one yt-dlp `--list-formats` preflight and logs compact output for diagnostics.
fn log_ytdlp_list_formats_once(
    job_id: i32,
    url: &str,
    cookie_path: Option<&Path>,
    polite_mode: bool,
    mode_name: &str,
    profile_name: &str,
    extractor_args: Option<&str>,
) {
    let mut args = build_ytdlp_download_args_with_mode(
        url,
        "/tmp/unused.%(ext)s",
        cookie_path,
        None,
        polite_mode,
        extractor_args,
    );

    args.retain(|arg| arg != "--output");
    args.retain(|arg| arg != "/tmp/unused.%(ext)s");
    args.push("--list-formats".to_string());

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    let mut command = Command::new("yt-dlp");
    command.args(&arg_refs);

    match command.output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

            if output.status.success() {
                if stdout.is_empty() {
                    log_job_info(&format!(
                        "job {} mode '{}' profile '{}' list-formats succeeded with empty output",
                        job_id, mode_name, profile_name
                    ));
                } else {
                    log_job_info(&format!(
                        "job {} mode '{}' profile '{}' list-formats output:\n{}",
                        job_id,
                        mode_name,
                        profile_name,
                        truncate_log_text(&stdout, YTDLP_LIST_FORMATS_LOG_MAX_CHARS)
                    ));
                }
                return;
            }

            let formatted = CommandFailure::from_process_output(
                "yt-dlp",
                &arg_refs,
                output.status.to_string(),
                stdout,
                stderr,
            );

            log_job_info(&format!(
                "job {} mode '{}' profile '{}' list-formats failed: {}",
                job_id, mode_name, profile_name, formatted
            ));
        }
        Err(error) => {
            log_job_info(&format!(
                "job {} mode '{}' profile '{}' list-formats execution failed: {}",
                job_id, mode_name, profile_name, error
            ));
        }
    }
}

fn truncate_log_text(source: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for (index, character) in source.chars().enumerate() {
        if index >= max_chars {
            output.push_str("\n...<truncated>");
            break;
        }
        output.push(character);
    }
    output
}

/// Builds yt-dlp download arguments for one attempt and one network behavior mode.
fn build_ytdlp_download_args_with_mode(
    url: &str,
    output_template: &str,
    cookie_path: Option<&Path>,
    format_selector: Option<&str>,
    polite_mode: bool,
    extractor_args: Option<&str>,
) -> Vec<String> {
    let mut args = vec!["--no-warnings".to_string(), "--no-playlist".to_string()];

    if let Some(value) = extractor_args {
        args.push("--extractor-args".to_string());
        args.push(value.to_string());
    }

    if polite_mode {
        args.push("--sleep-requests".to_string());
        args.push(YTDLP_POLITE_SLEEP_REQUESTS.to_string());
        args.push("--sleep-interval".to_string());
        args.push(YTDLP_POLITE_SLEEP_INTERVAL.to_string());
        args.push("--max-sleep-interval".to_string());
        args.push(YTDLP_POLITE_MAX_SLEEP_INTERVAL.to_string());
        args.push("--concurrent-fragments".to_string());
        args.push("1".to_string());
        args.push("--limit-rate".to_string());
        args.push(YTDLP_POLITE_LIMIT_RATE.to_string());
        args.push("--retries".to_string());
        args.push("10".to_string());
        args.push("--fragment-retries".to_string());
        args.push("10".to_string());
        args.push("--extractor-retries".to_string());
        args.push("5".to_string());
    }

    if let Some(selector) = format_selector {
        args.push("--format".to_string());
        args.push(selector.to_string());
    }

    args.push("--output".to_string());
    args.push(output_template.to_string());

    if let Some(path) = cookie_path {
        args.push("--cookies".to_string());
        args.push(path.to_string_lossy().to_string());
    }

    args.push(url.to_string());
    args
}

/// Queries yt-dlp JSON metadata and picks the best currently available format id.
fn find_best_available_ytdlp_format_id(
    url: &str,
    cookie_path: Option<&Path>,
    extractor_args: Option<&str>,
) -> MyRes<Option<String>> {
    let mut args = vec![
        "--no-warnings".to_string(),
        "--no-playlist".to_string(),
        "--dump-single-json".to_string(),
        "--no-download".to_string(),
    ];

    if let Some(value) = extractor_args {
        args.push("--extractor-args".to_string());
        args.push(value.to_string());
    }

    if let Some(path) = cookie_path {
        args.push("--cookies".to_string());
        args.push(path.to_string_lossy().to_string());
    }

    args.push(url.to_string());

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut command = Command::new("yt-dlp");
    command.args(&arg_refs);
    let output = command
        .output()
        .map_err(|error| format!("format probe command execution failed: {error}"))?;

    if !output.status.success() {
        return Err(CommandFailure::from_process_output(
            "yt-dlp",
            &arg_refs,
            output.status.to_string(),
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        )
        .to_string()
        .into());
    }

    let payload = String::from_utf8_lossy(&output.stdout).to_string();
    let parsed: YtdlpVideoMetadata = serde_json::from_str(&payload)
        .map_err(|error| format!("failed to parse yt-dlp format probe JSON: {error}"))?;

    Ok(select_best_format_id(&parsed.formats))
}

#[derive(Debug, Deserialize)]
struct YtdlpVideoMetadata {
    #[serde(default)]
    formats: Vec<YtdlpFormatEntry>,
}

#[derive(Debug, Deserialize)]
struct YtdlpFormatEntry {
    format_id: String,
    #[serde(default)]
    ext: String,
    #[serde(default)]
    acodec: String,
    #[serde(default)]
    vcodec: String,
    height: Option<i32>,
    tbr: Option<f64>,
    abr: Option<f64>,
}

/// Picks a stable best-effort format id for audio-first mp3 conversion.
fn select_best_format_id(formats: &[YtdlpFormatEntry]) -> Option<String> {
    pick_best_format_id(formats, |format| {
        has_audio_codec(format)
            && !has_video_codec(format)
            && is_likely_mp3_convertible_audio_format(format)
    })
    .or_else(|| {
        pick_best_format_id(formats, |format| {
            has_audio_codec(format) && is_likely_mp3_convertible_audio_format(format)
        })
    })
    .or_else(|| {
        pick_best_format_id(formats, |format| {
            has_audio_codec(format) && !has_video_codec(format)
        })
    })
    .or_else(|| pick_best_format_id(formats, has_audio_codec))
}

fn pick_best_format_id<P>(formats: &[YtdlpFormatEntry], predicate: P) -> Option<String>
where
    P: Fn(&YtdlpFormatEntry) -> bool,
{
    formats
        .iter()
        .filter(|format| !format.format_id.is_empty())
        .filter(|format| predicate(format))
        .max_by(|left, right| score_format(left).cmp(&score_format(right)))
        .map(|format| format.format_id.clone())
}

fn has_audio_codec(format: &YtdlpFormatEntry) -> bool {
    !format.acodec.is_empty() && format.acodec != "none"
}

fn has_video_codec(format: &YtdlpFormatEntry) -> bool {
    !format.vcodec.is_empty() && format.vcodec != "none"
}

/// Approximates whether ffmpeg can reasonably transcode this source to mp3.
fn is_likely_mp3_convertible_audio_format(format: &YtdlpFormatEntry) -> bool {
    let ext = format.ext.as_str();
    let codec = format.acodec.as_str();

    if matches!(ext, "m4a" | "mp3" | "webm" | "ogg" | "opus" | "aac") {
        return has_audio_codec(format);
    }

    matches!(
        codec,
        "mp4a.40.2" | "mp4a" | "aac" | "opus" | "vorbis" | "mp3"
    )
}

fn score_format(format: &YtdlpFormatEntry) -> i64 {
    let abr = format.abr.unwrap_or(0.0).round() as i64;
    let tbr = format.tbr.unwrap_or(0.0).round() as i64;
    let height = i64::from(format.height.unwrap_or(0));
    let video_penalty = if has_video_codec(format) {
        -1_000_000
    } else {
        0
    };
    let container_bonus = match format.ext.as_str() {
        "m4a" | "mp3" | "webm" | "ogg" | "opus" | "aac" => 1_000,
        _ => 0,
    };
    (abr * 1_000) + (tbr * 10) + height + container_bonus + video_penalty
}

/// Locates the downloaded media artifact produced by yt-dlp in the job temp directory.
fn find_downloaded_file_in_temp_dir(temp_dir: &Path) -> MyRes<PathBuf> {
    let entries = fs::read_dir(temp_dir)?;
    let mut candidates: Vec<PathBuf> = Vec::new();

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(filename) = path.file_name().and_then(|part| part.to_str()) else {
            continue;
        };

        if !filename.starts_with("downloaded.") {
            continue;
        }

        if filename.ends_with(".part") || filename.ends_with(".ytdl") {
            continue;
        }

        candidates.push(path);
    }

    let preferred_mp3 = temp_dir.join("downloaded.mp3");
    if preferred_mp3.exists() {
        return Ok(preferred_mp3);
    }

    if candidates.is_empty() {
        return Err(format!(
            "yt-dlp finished but no downloaded file found in {}",
            temp_dir.display()
        )
        .into());
    }

    candidates.sort();
    Ok(candidates.swap_remove(0))
}

/// Ensures the downloaded artifact is mp3 so downstream processing can run reliably.
fn run_ensure_mp3_step(
    job_id: i32,
    attempt_count: i32,
    temp_dir: &Path,
    downloaded_path: &mut String,
) -> MyRes<()> {
    start_step(job_id, attempt_count, JobStep::EnsureMp3)?;

    if downloaded_path.trim().is_empty() {
        return Err("downloaded_path is empty before ensure_mp3".into());
    }

    let source_path = Path::new(downloaded_path);
    if !source_path.exists() {
        return Err(format!("downloaded file missing before ensure_mp3: {downloaded_path}").into());
    }

    let extension = source_path
        .extension()
        .and_then(|part| part.to_str())
        .unwrap_or_default()
        .to_lowercase();

    if extension == "mp3" {
        update_job_paths(
            job_id,
            temp_dir.to_str().unwrap_or_default(),
            downloaded_path,
            "",
            "",
        )?;
        finish_step_with_note(job_id, JobStep::EnsureMp3, "already mp3");
        return Ok(());
    }

    let converted_path = temp_dir.join("downloaded.mp3");
    run_command_capture(
        "ffmpeg",
        &[
            "-hide_banner",
            "-y",
            "-i",
            downloaded_path,
            "-vn",
            "-codec:a",
            "libmp3lame",
            "-q:a",
            "2",
            converted_path.to_str().unwrap_or_default(),
        ],
    )?;

    if !converted_path.exists() {
        return Err(format!(
            "ffmpeg mp3 conversion output is missing: {}",
            converted_path.display()
        )
        .into());
    }

    *downloaded_path = converted_path.to_string_lossy().to_string();
    update_job_paths(
        job_id,
        temp_dir.to_str().unwrap_or_default(),
        downloaded_path,
        "",
        "",
    )?;
    finish_step_with_note(job_id, JobStep::EnsureMp3, "converted to mp3");

    Ok(())
}

/// Runs mp3gain normalization and updates normalized path metadata.
fn run_mp3gain_step(
    job_id: i32,
    attempt_count: i32,
    downloaded_path: &str,
    normalized_path: &mut String,
) -> MyRes<()> {
    start_step(job_id, attempt_count, JobStep::Mp3Gain)?;

    if downloaded_path.trim().is_empty() {
        return Err("downloaded_path is empty before mp3gain".into());
    }

    if !Path::new(downloaded_path).exists() {
        return Err(format!("downloaded file missing before mp3gain: {downloaded_path}").into());
    }

    run_command_capture("mp3gain", &["-r", "-d", "10", "-k", downloaded_path])?;

    *normalized_path = downloaded_path.to_string();
    let temp_dir = get_job_temp_dir_from_path(downloaded_path);
    update_job_paths(
        job_id,
        temp_dir.to_str().unwrap_or_default(),
        downloaded_path,
        normalized_path,
        "",
    )?;
    finish_step(job_id, JobStep::Mp3Gain);
    Ok(())
}

/// Runs ffmpeg compand adjustment and returns the adjusted path.
fn run_ffmpeg_adjust_step(
    job_id: i32,
    attempt_count: i32,
    temp_dir: &Path,
    normalized_path: &str,
) -> MyRes<PathBuf> {
    start_step(job_id, attempt_count, JobStep::FfmpegAdjust)?;

    let adjusted_path = temp_dir.join("adjusted.mp3");
    if adjusted_path.exists() {
        finish_step_with_note(job_id, JobStep::FfmpegAdjust, "skipped, already adjusted");
        return Ok(adjusted_path);
    }

    run_command_capture(
        "ffmpeg",
        &[
            "-hide_banner",
            "-y",
            "-i",
            normalized_path,
            "-af",
            FFMPEG_COMPAND_FILTER,
            adjusted_path.to_str().unwrap_or_default(),
        ],
    )?;

    if !adjusted_path.exists() {
        return Err(format!("ffmpeg output is missing: {}", adjusted_path.display()).into());
    }

    finish_step(job_id, JobStep::FfmpegAdjust);
    Ok(adjusted_path)
}

/// Renames the adjusted file in temp dir to a deterministic metadata-based filename.
fn run_rename_step(
    job: &Job,
    attempt_count: i32,
    temp_dir: &Path,
    adjusted_path: &Path,
) -> MyRes<PathBuf> {
    start_step(job.id, attempt_count, JobStep::Rename)?;

    let base_for_fallback = adjusted_path
        .file_stem()
        .and_then(|part| part.to_str())
        .unwrap_or("download");
    let target_name = build_target_filename(job, base_for_fallback);
    let renamed_path = temp_dir.join(target_name);

    if renamed_path.exists() {
        finish_step_with_note(job.id, JobStep::Rename, "skipped, already renamed");
        return Ok(renamed_path);
    }

    if !adjusted_path.exists() {
        return Err(format!(
            "adjusted file missing before rename: {}",
            adjusted_path.display()
        )
        .into());
    }

    fs::rename(adjusted_path, &renamed_path)?;
    finish_step(job.id, JobStep::Rename);
    Ok(renamed_path)
}

/// Moves the renamed file into the music directory and stores final path.
fn run_move_step(
    job: &Job,
    attempt_count: i32,
    renamed_path: &Path,
    downloaded_path: &str,
    normalized_path: &str,
    final_path: &mut String,
) -> MyRes<()> {
    start_step(job.id, attempt_count, JobStep::MoveToMusicDir)?;

    if !final_path.trim().is_empty() && Path::new(final_path).exists() {
        finish_step_with_note(job.id, JobStep::MoveToMusicDir, "skipped, already moved");
        return Ok(());
    }

    let music_dir = get_music_dir();
    fs::create_dir_all(&music_dir)?;

    let filename = renamed_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("downloaded.mp3");

    let mut target_path = music_dir.join(filename);
    if target_path.exists() {
        let stem = renamed_path
            .file_stem()
            .and_then(|part| part.to_str())
            .unwrap_or("downloaded");
        let extension = renamed_path
            .extension()
            .and_then(|part| part.to_str())
            .unwrap_or("mp3");
        let collision_name = format!("{}_{}.{}", stem, job.id, extension);
        target_path = music_dir.join(collision_name);
    }

    move_file(renamed_path, &target_path)?;
    *final_path = target_path.to_string_lossy().to_string();

    let temp_dir = ensure_temp_dir_for_job(job.id)?;
    update_job_paths(
        job.id,
        temp_dir.to_str().unwrap_or_default(),
        downloaded_path,
        normalized_path,
        final_path,
    )?;
    finish_step(job.id, JobStep::MoveToMusicDir);

    Ok(())
}

/// Imports final file into songs DB and returns song id.
fn run_import_step(job_id: i32, attempt_count: i32) -> MyRes<i32> {
    start_step(job_id, attempt_count, JobStep::ImportDb)?;
    let song_id = import_job_song(job_id)?;
    finish_step(job_id, JobStep::ImportDb);
    Ok(song_id)
}

/// Marks a job as failed for one concrete state-machine step.
fn fail_job_with_step(job_id: i32, step: JobStep, error: Box<dyn std::error::Error>) -> MyRes<()> {
    let message = truncate_error_message(&error.to_string());
    mark_job_failed(job_id, step, &message)?;
    log_job_error(&format!("job {job_id} failed at {:?}: {message}", step));
    Ok(())
}

/// Runs one external command and returns a rich error when it fails.
fn run_command_capture(program: &str, args: &[&str]) -> MyRes<()> {
    run_command_capture_detailed(program, args).map_err(|error| error.into())
}

/// Runs one external command and preserves stdout/stderr for targeted retry handling.
fn run_command_capture_detailed(program: &str, args: &[&str]) -> Result<(), CommandFailure> {
    let mut command = Command::new(program);
    command.args(args);
    let output = command.output().map_err(CommandFailure::from_io)?;
    if output.status.success() {
        return Ok(());
    }

    Err(CommandFailure::from_process_output(
        program,
        args,
        output.status.to_string(),
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
        String::from_utf8_lossy(&output.stderr).trim().to_string(),
    ))
}

#[derive(Debug)]
struct CommandFailure {
    formatted: String,
    stdout: String,
    stderr: String,
}

impl CommandFailure {
    /// Creates a command failure from a process exit result.
    fn from_process_output(
        program: &str,
        args: &[&str],
        status: String,
        stdout: String,
        stderr: String,
    ) -> Self {
        let formatted = format!(
            "command failed: {} {} | status: {} | stdout: {} | stderr: {}",
            program,
            args.join(" "),
            status,
            stdout,
            stderr
        );

        Self {
            formatted: truncate_error_message(&formatted),
            stdout,
            stderr,
        }
    }

    /// Creates a command failure from an I/O error before the process could run.
    fn from_io(error: std::io::Error) -> Self {
        let formatted = format!("command execution failed: {error}");
        Self {
            formatted: truncate_error_message(&formatted),
            stdout: String::new(),
            stderr: error.to_string(),
        }
    }

    /// Reports whether yt-dlp rejected the requested format selector.
    fn is_requested_format_unavailable(&self) -> bool {
        self.stderr.contains("Requested format is not available")
            || self.stdout.contains("Requested format is not available")
    }
}

impl std::fmt::Display for CommandFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.formatted)
    }
}

impl std::error::Error for CommandFailure {}

/// Creates and returns deterministic temp dir for one queue job.
fn ensure_temp_dir_for_job(job_id: i32) -> MyRes<PathBuf> {
    let base = get_upload_dir();
    let dir = base.join("jobs").join(job_id.to_string());
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Returns the configured upload directory used by queue job temp files.
fn get_upload_dir() -> PathBuf {
    let env_value = env::var("UPLOADDIR");
    if let Ok(value) = env_value {
        return PathBuf::from(value);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("music")
        .join("upload")
}

/// Returns the directory where per-job cookie text files are stored.
fn get_cookie_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("cookies")
}

/// Builds the deterministic cookie file path for one queue job.
fn cookie_file_path_for_job(job_id: i32) -> PathBuf {
    let filename = format!("job-{job_id}.txt");
    get_cookie_dir().join(filename)
}

/// Returns sanitized cookie payload text or `None` when no cookie input was provided.
fn extract_cookie_payload(raw: Option<&str>) -> Option<String> {
    let value = raw?;

    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}

/// Validates cookie payload size to prevent oversized untrusted request bodies.
fn validate_cookie_payload_len(cookie_text: &str) -> MyRes<()> {
    if cookie_text.len() > JOB_COOKIE_MAX_LEN {
        return Err(
            format!("Cookie input exceeds maximum size of {JOB_COOKIE_MAX_LEN} bytes.").into(),
        );
    }

    Ok(())
}

/// Persists a per-job cookie text file in `/cookies` without storing cookie data in the database.
fn write_job_cookie_file(job_id: i32, cookie_text: &str) -> MyRes<()> {
    validate_cookie_payload_len(cookie_text)?;

    let dir = get_cookie_dir();
    fs::create_dir_all(&dir)?;

    let file_path = cookie_file_path_for_job(job_id);
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }

    let mut file = options.open(file_path)?;
    file.write_all(cookie_text.as_bytes())?;

    if !cookie_text.ends_with('\n') {
        file.write_all(b"\n")?;
    }

    Ok(())
}

/// Deletes per-job cookie file if present so credentials are short-lived on disk.
fn delete_job_cookie_file_if_exists(job_id: i32) -> MyRes<()> {
    let file_path = cookie_file_path_for_job(job_id);
    let result = fs::remove_file(file_path);
    if let Err(err) = result {
        if err.kind() == ErrorKind::NotFound {
            return Ok(());
        }

        return Err(err.into());
    }

    Ok(())
}

/// Deletes the per-job upload temp directory before the job is marked completed.
fn delete_job_upload_dir_if_exists(job_id: i32) -> MyRes<()> {
    let dir_path = get_upload_dir().join("jobs").join(job_id.to_string());
    let result = fs::remove_dir_all(dir_path);
    if let Err(err) = result {
        if err.kind() == ErrorKind::NotFound {
            return Ok(());
        }

        return Err(err.into());
    }

    Ok(())
}

/// Returns the configured music directory where final mp3 files are stored.
fn get_music_dir() -> PathBuf {
    let env_value = env::var("MUSICDIR");
    if let Ok(value) = env_value {
        return PathBuf::from(value);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("music")
}

/// Converts arbitrary metadata into a filesystem-safe filename component.
fn sanitize_filename_part(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric()
            || character == ' '
            || character == '_'
            || character == '-'
        {
            output.push(character);
            continue;
        }

        output.push('_');
    }

    let trimmed = output.trim();
    if trimmed.is_empty() {
        return "song".to_string();
    }

    trimmed.to_string()
}

/// Builds a deterministic final filename from job metadata and fallback base text.
fn build_target_filename(job: &Job, fallback_base: &str) -> String {
    let songname = sanitize_song_field(&job.songname, fallback_base);
    let artist = sanitize_song_field(&job.artist, "unknown_artist");
    let album = sanitize_song_field(&job.album, "unknown_album");

    let safe_songname = sanitize_filename_part(&songname);
    let safe_artist = sanitize_filename_part(&artist);
    let safe_album = sanitize_filename_part(&album);

    format!("{} - {} - {}.mp3", safe_artist, safe_album, safe_songname)
}

/// Moves a file, falling back to copy+delete when rename crosses filesystems.
fn move_file(source: &Path, destination: &Path) -> MyRes<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(source, destination)?;
            fs::remove_file(source)?;
            Ok(())
        }
    }
}

/// Derives job temp dir from an existing file path.
fn get_job_temp_dir_from_path(path: &str) -> PathBuf {
    let source = Path::new(path);
    if let Some(parent) = source.parent() {
        return parent.to_path_buf();
    }
    get_upload_dir().join("jobs")
}

/// Logs startup warnings when required external processing tools are missing.
fn log_tool_availability_warnings() {
    let required_tools = ["yt-dlp", "mp3gain", "ffmpeg"];
    for tool in required_tools {
        let output = Command::new(tool).arg("--version").output();
        if output.is_ok() {
            continue;
        }

        log_job_error(&format!(
            "required tool '{tool}' is not available in PATH; related jobs will fail"
        ));
    }
}

/// Truncates stored errors so table rows remain readable and bounded.
fn truncate_error_message(message: &str) -> String {
    let mut output = String::new();
    for (index, character) in message.chars().enumerate() {
        if index >= JOB_ERROR_MAX_LEN {
            output.push_str("...");
            break;
        }
        output.push(character);
    }
    output
}

/// Emits an info-level message for download queue activity.
fn log_job_info(message: &str) {
    println!("[jobs] {message}");
}

/// Emits an error-level message for download queue failures.
fn log_job_error(message: &str) {
    eprintln!("[jobs] {message}");
}

#[cfg(test)]
mod tests {
    use super::{
        build_ytdlp_download_args_with_mode, cookie_file_path_for_job, extract_cookie_payload,
        is_job_cookie_filename, sanitize_filename_part, sanitize_job_url, select_best_format_id,
        truncate_error_message, validate_cookie_payload_len, JOB_COOKIE_MAX_LEN,
        YTDLP_YOUTUBE_EXTRACTOR_ARGS,
    };
    use std::path::Path;

    #[derive(Debug)]
    struct FormatSeed<'a> {
        id: &'a str,
        ext: &'a str,
        acodec: &'a str,
        vcodec: &'a str,
        height: Option<i32>,
        tbr: Option<f64>,
        abr: Option<f64>,
    }

    fn seed_format(seed: FormatSeed<'_>) -> super::YtdlpFormatEntry {
        super::YtdlpFormatEntry {
            format_id: seed.id.to_string(),
            ext: seed.ext.to_string(),
            acodec: seed.acodec.to_string(),
            vcodec: seed.vcodec.to_string(),
            height: seed.height,
            tbr: seed.tbr,
            abr: seed.abr,
        }
    }

    #[test]
    fn truncates_long_job_error_messages() {
        let source = "x".repeat(1200);
        let output = truncate_error_message(&source);
        assert!(output.len() <= 1003);
        assert!(output.ends_with("..."));
    }

    #[test]
    fn keeps_short_job_error_messages_unchanged() {
        let source = "short error";
        let output = truncate_error_message(source);
        assert_eq!(output, source);
    }

    #[test]
    fn sanitizes_filename_part() {
        let output = sanitize_filename_part("a/b:c*name");
        assert_eq!(output, "a_b_c_name");
    }

    #[test]
    fn builds_cookie_file_path_for_job() {
        let path = cookie_file_path_for_job(42);
        let as_text = path.to_string_lossy();
        assert!(as_text.ends_with("/cookies/job-42.txt"));
    }

    #[test]
    fn extracts_cookie_payload_only_when_non_empty() {
        assert!(extract_cookie_payload(None).is_none());
        assert!(extract_cookie_payload(Some("   \n\t")).is_none());
        let cookie = extract_cookie_payload(Some("  # Netscape cookie  "));
        assert_eq!(cookie, Some("# Netscape cookie".to_string()));
    }

    #[test]
    fn rejects_oversized_cookie_payload() {
        let oversized = "x".repeat(JOB_COOKIE_MAX_LEN + 1);
        let result = validate_cookie_payload_len(&oversized);
        assert!(result.is_err());
    }

    #[test]
    fn recognizes_job_cookie_filename_pattern() {
        assert!(is_job_cookie_filename("job-1.txt"));
        assert!(is_job_cookie_filename("job-123456.txt"));
        assert!(!is_job_cookie_filename("job-.txt"));
        assert!(!is_job_cookie_filename("job-12.csv"));
        assert!(!is_job_cookie_filename("cookies.txt"));
        assert!(!is_job_cookie_filename("job-a.txt"));
    }

    #[test]
    fn builds_ytdlp_args_with_format_and_cookies() {
        let args = build_ytdlp_download_args_with_mode(
            "https://example.com/watch?v=1",
            "/tmp/downloaded.%(ext)s",
            Some(Path::new("/tmp/job-cookie.txt")),
            Some("bestaudio/best"),
            false,
            Some(YTDLP_YOUTUBE_EXTRACTOR_ARGS),
        );

        assert_eq!(
            args,
            vec![
                "--no-warnings",
                "--no-playlist",
                "--extractor-args",
                "youtube:player_client=default,-tv,-tv_downgraded",
                "--format",
                "bestaudio/best",
                "--output",
                "/tmp/downloaded.%(ext)s",
                "--cookies",
                "/tmp/job-cookie.txt",
                "https://example.com/watch?v=1",
            ]
        );
    }

    #[test]
    fn builds_ytdlp_args_without_format_when_using_default_selector() {
        let args = build_ytdlp_download_args_with_mode(
            "https://example.com/watch?v=1",
            "/tmp/downloaded.%(ext)s",
            None,
            None,
            false,
            Some(YTDLP_YOUTUBE_EXTRACTOR_ARGS),
        );

        assert_eq!(
            args,
            vec![
                "--no-warnings",
                "--no-playlist",
                "--extractor-args",
                "youtube:player_client=default,-tv,-tv_downgraded",
                "--output",
                "/tmp/downloaded.%(ext)s",
                "https://example.com/watch?v=1",
            ]
        );
    }

    #[test]
    fn prefers_best_audio_only_format_over_video_formats() {
        let formats = vec![
            seed_format(FormatSeed {
                id: "22",
                ext: "mp4",
                acodec: "mp4a.40.2",
                vcodec: "avc1",
                height: Some(720),
                tbr: Some(1400.0),
                abr: Some(128.0),
            }),
            seed_format(FormatSeed {
                id: "251",
                ext: "webm",
                acodec: "opus",
                vcodec: "none",
                height: None,
                tbr: Some(160.0),
                abr: Some(160.0),
            }),
            seed_format(FormatSeed {
                id: "140",
                ext: "m4a",
                acodec: "mp4a.40.2",
                vcodec: "none",
                height: None,
                tbr: Some(128.0),
                abr: Some(128.0),
            }),
        ];

        let selected = select_best_format_id(&formats);
        assert_eq!(selected, Some("251".to_string()));
    }

    #[test]
    fn falls_back_to_muxed_when_only_video_plus_audio_exists() {
        let formats = vec![seed_format(FormatSeed {
            id: "18",
            ext: "mp4",
            acodec: "mp4a.40.2",
            vcodec: "avc1",
            height: Some(360),
            tbr: Some(600.0),
            abr: Some(96.0),
        })];

        let selected = select_best_format_id(&formats);
        assert_eq!(selected, Some("18".to_string()));
    }

    #[test]
    fn falls_back_to_best_audio_only_format_when_needed() {
        let formats = vec![
            seed_format(FormatSeed {
                id: "140",
                ext: "m4a",
                acodec: "mp4a.40.2",
                vcodec: "none",
                height: None,
                tbr: Some(128.0),
                abr: Some(128.0),
            }),
            seed_format(FormatSeed {
                id: "251",
                ext: "webm",
                acodec: "opus",
                vcodec: "none",
                height: None,
                tbr: Some(160.0),
                abr: Some(160.0),
            }),
        ];

        let selected = select_best_format_id(&formats);
        assert_eq!(selected, Some("140".to_string()));
    }

    #[test]
    fn sanitizes_youtube_watch_link_by_removing_playlist_params() {
        let input = "https://www.youtube.com/watch?v=84EEjgSEOFM&list=RDu9MvjaG0OQY&index=29";
        let output = sanitize_job_url(input);
        assert_eq!(output, "https://www.youtube.com/watch?v=84EEjgSEOFM");
    }

    #[test]
    fn sanitizes_youtu_be_link_by_removing_tracking_params() {
        let input = "https://youtu.be/Rf_TfHK3-3g?si=Bcjsy1kaATTg_4Uy";
        let output = sanitize_job_url(input);
        assert_eq!(output, "https://www.youtube.com/watch?v=Rf_TfHK3-3g");
    }
}
