//! CSV log of when each line was told to the overlay, written only when `LARIK_TIMING_LOG` names a file.

use std::fs::OpenOptions;
use std::io::Write;

pub const ENV: &str = "LARIK_TIMING_LOG";
pub const HEADER: &str = "kind,wall_ms,app,title,line_index,lrc_ms,pos_ms,extra";

pub struct TimingLog {
    out: Box<dyn Write + Send>,
}

impl TimingLog {
    /// None when the variable is not set or the file cannot be opened. Appends; the header is written to an empty file.
    pub fn from_env() -> Option<TimingLog> {
        let path = std::env::var_os(ENV).filter(|p| !p.is_empty())?;
        let mut file = OpenOptions::new().create(true).append(true).open(path).ok()?;
        if file.metadata().ok()?.len() == 0 {
            writeln!(file, "{HEADER}").ok()?;
        }
        Some(TimingLog { out: Box::new(file) })
    }

    pub fn new(out: Box<dyn Write + Send>) -> TimingLog {
        TimingLog { out }
    }

    /// One row, flushed at once. A write error is ignored: the log must never stop the lyrics.
    #[allow(clippy::too_many_arguments)]
    pub fn row(
        &mut self,
        kind: &str,
        wall_ms: i64,
        app: &str,
        title: &str,
        line_index: Option<usize>,
        lrc_ms: Option<u32>,
        pos_ms: i64,
        extra: &str,
    ) {
        let cell = |n: Option<String>| n.unwrap_or_default();
        let row = [
            kind.to_string(),
            wall_ms.to_string(),
            csv(app),
            csv(title),
            cell(line_index.map(|i| i.to_string())),
            cell(lrc_ms.map(|t| t.to_string())),
            pos_ms.to_string(),
            csv(extra),
        ]
        .join(",");
        let _ = writeln!(self.out, "{row}").and_then(|()| self.out.flush());
    }
}

/// Quoted when it holds a comma, a quote or a line break.
fn csv(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\"").replace(['\r', '\n'], " "))
    } else {
        field.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn rows_have_eight_columns_and_empty_cells_for_none() {
        let out = Shared::default();
        let mut log = TimingLog::new(Box::new(out.clone()));
        log.row("emit", 1_700_000_000_123, "Spotify.exe", "Yellow", Some(3), Some(12_340), 12_361, "7");
        log.row("sync", 5, "", "Yellow", None, None, -250, "seek");
        let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        assert_eq!(text, "emit,1700000000123,Spotify.exe,Yellow,3,12340,12361,7\nsync,5,,Yellow,,,-250,seek\n");
    }

    #[test]
    fn titles_with_commas_quotes_and_line_breaks_stay_in_one_cell() {
        let out = Shared::default();
        TimingLog::new(Box::new(out.clone())).row("emit", 1, "a", "Hello, \"World\"\nagain", None, None, 0, "");
        let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        assert_eq!(text, "emit,1,a,\"Hello, \"\"World\"\" again\",,,0,\n");
    }
}
