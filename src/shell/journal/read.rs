//! Reading the log back, tolerating the one kind of damage a crash can
//! leave: a torn final line.

use crate::core::error::AlmanacError;
use crate::core::journal::Record;

use super::Journal;

impl Journal {
    /// Reads every intact record from the log.
    ///
    /// A trailing partial line — the signature of a crash mid-append —
    /// is skipped rather than treated as corruption: the record it
    /// would have become was never acknowledged to its source, so
    /// dropping it loses nothing that was ever promised. Any *earlier*
    /// unparseable line is a real integrity problem and is surfaced.
    pub(super) fn read_records(&self) -> Result<Vec<Record>, AlmanacError> {
        let contents = match std::fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(AlmanacError::Config {
                    message: format!("failed to read journal {}: {e}", self.path.display()),
                    remedy: format!("check permissions on {}", self.path.display()),
                });
            }
        };

        let ends_cleanly = contents.is_empty() || contents.ends_with('\n');
        let lines: Vec<&str> = contents.lines().collect();

        let mut records = Vec::with_capacity(lines.len());
        for (i, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Record>(line) {
                Ok(record) => records.push(record),
                Err(e) => {
                    let is_trailing_partial = !ends_cleanly && i + 1 == lines.len();
                    if is_trailing_partial {
                        tracing::warn!(
                            journal = %self.path.display(),
                            "discarding a torn final journal line — it was never acknowledged to its source"
                        );
                        continue;
                    }
                    return Err(AlmanacError::Config {
                        message: format!(
                            "journal {} line {} is unparseable: {e}",
                            self.path.display(),
                            i + 1
                        ),
                        remedy: "the journal is damaged beyond the usual torn-tail case; move it \
                                 aside to inspect before restarting, since doing so abandons any \
                                 undelivered events it holds"
                            .to_string(),
                    });
                }
            }
        }

        Ok(records)
    }
}
