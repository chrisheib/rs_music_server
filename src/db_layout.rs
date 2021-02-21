use crate::db::db_execute;
use stable_eyre::Result;

fn create_table() -> Result<()> {
    db_execute("DROP TABLE IF EXISTS songs;")?;
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
    )?;
    Ok(())
}

pub fn patch_layout() -> actix_web::Result<(), actix_web::Error> {
    crate::errconv(create_table())
}
