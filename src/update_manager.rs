use crate::{
    db::{db_execute, db_str_read, db_uint32_read},
    MyRes,
};
use stable_eyre::eyre::eyre;

pub fn db_update() -> MyRes<()> {
    loop {
        let table_exists = db_uint32_read(
            "SELECT count(name) FROM sqlite_master WHERE type='table' AND name='config'",
        )?;
        let config_exists = table_exists > 0;

        if !config_exists {
            let table_exists = db_uint32_read(
                "SELECT count(name) FROM sqlite_master WHERE type='table' AND name='songs'",
            )?;
            if table_exists == 0 {
                // gibt noch nix, alles erstellen:
                v0()?;
            } else {
                // songs sind vorhanden, config aber noch nicht.
                v1()?;
            }
        } else {
            // config und songs sind vorhanden, normalen updateprozess starten.
            let version = db_str_read("select value from config where key = 'version'")?;
            match version.as_str() {
                "2" => v2()?,
                "3" => v3()?,
                "4" => v4()?,
                "5" => v5()?,
                "6" => break,
                _ => Err(eyre!("Unbekannte Versionsnummer!"))?,
            }
        }
    }

    Ok(())
}

fn v0() -> MyRes<()> {
    db_execute("DROP TABLE IF EXISTS songs;", [])?;
    db_execute(
        "CREATE TABLE songs (
        id INTEGER not null primary key autoincrement,
        path TEXT unique,
        filename TEXT,
        songname TEXT,
        artist TEXT,
        album TEXT,
        length TEXT,
        seconds INTEGER,
        rating INTEGER,
        vote INTEGER
    );",
        [],
    )
}

fn v1() -> MyRes<()> {
    db_execute(
        "CREATE TABLE config (
        key TEXT unique primary key,
        value TEXT
    );",
        [],
    )?;
    db_execute(
        "INSERT INTO config (key, value) values ('version', '2')",
        [],
    )?;
    db_execute(
        "ALTER TABLE songs ADD COLUMN deleted INTEGER DEFAULT 0 NOT NULL",
        [],
    )
}

fn v2() -> MyRes<()> {
    db_execute(
        "ALTER TABLE songs ADD COLUMN times_played INTEGER DEFAULT 0 NOT NULL",
        [],
    )?;
    db_execute("UPDATE config SET value = '3' WHERE key LIKE 'version'", [])
}

fn v3() -> MyRes<()> {
    db_execute("UPDATE songs SET rating = 0 WHERE rating < 100", [])?;
    db_execute("UPDATE songs SET rating = 1 WHERE rating = 100", [])?;
    db_execute("UPDATE songs SET rating = 2 WHERE rating = 200", [])?;
    db_execute("UPDATE songs SET rating = 3 WHERE rating = 400", [])?;
    db_execute("UPDATE songs SET rating = 4 WHERE rating = 800", [])?;
    db_execute("UPDATE songs SET rating = 5 WHERE rating = 1600", [])?;
    db_execute("UPDATE songs SET rating = 6 WHERE rating = 3200", [])?;
    db_execute("UPDATE songs SET rating = 7 WHERE rating >= 6400", [])?;
    db_execute("UPDATE config SET value = '4' WHERE key LIKE 'version'", [])
}

fn v4() -> MyRes<()> {
    db_execute(
        "CREATE TABLE IF NOT EXISTS download_jobs (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        completed_at TEXT,
        url TEXT NOT NULL,
        songname TEXT NOT NULL DEFAULT '',
        artist TEXT NOT NULL DEFAULT '',
        album TEXT NOT NULL DEFAULT '',
        rating INTEGER NOT NULL DEFAULT 2,
        status TEXT NOT NULL,
        step TEXT NOT NULL,
        step_index INTEGER NOT NULL DEFAULT 0,
        attempt_count INTEGER NOT NULL DEFAULT 0,
        song_id INTEGER,
        error_message TEXT NOT NULL DEFAULT '',
        temp_dir TEXT NOT NULL DEFAULT '',
        downloaded_path TEXT NOT NULL DEFAULT '',
        normalized_path TEXT NOT NULL DEFAULT '',
        final_path TEXT NOT NULL DEFAULT ''
    );",
        [],
    )?;
    db_execute(
        "CREATE INDEX IF NOT EXISTS idx_download_jobs_status_created_at ON download_jobs (status, created_at)",
        [],
    )?;
    db_execute(
        "CREATE INDEX IF NOT EXISTS idx_download_jobs_completed_at ON download_jobs (completed_at)",
        [],
    )?;
    db_execute("UPDATE config SET value = '5' WHERE key LIKE 'version'", [])
}

fn v5() -> MyRes<()> {
    db_execute("ALTER TABLE songs ADD COLUMN last_played TEXT", [])?;
    db_execute(
        "CREATE INDEX IF NOT EXISTS idx_songs_last_played ON songs (last_played)",
        [],
    )?;
    db_execute("UPDATE config SET value = '6' WHERE key LIKE 'version'", [])
}
