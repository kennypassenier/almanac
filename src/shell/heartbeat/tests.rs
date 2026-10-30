use super::*;

/// Counts how many "alive" lines the task produced, by capturing
/// the tracing output rather than by trusting the code's own idea
/// of what it emitted.
#[derive(Clone, Default)]
struct Lines(Arc<std::sync::Mutex<Vec<String>>>);

impl std::io::Write for Lines {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap()
            .push(String::from_utf8_lossy(buf).to_string());
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Lines {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test(start_paused = true)]
async fn the_first_line_comes_after_one_interval_not_at_startup() {
    // The same shape of bug the updater had: a tokio interval fires
    // its first tick immediately, so a loop that does not consume
    // it logs at once — noise at exactly the moment the startup
    // lines have just said the same thing — and then drifts by one
    // period. Pinned here because it is invisible to every test
    // that calls the body directly.
    let lines = Lines::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(lines.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let (state, dir) = crate::shell::worker::tests::state_for_update_loop().await;
    let (tx, rx) = watch::channel(false);
    let every = Duration::from_secs(60);
    let task = tokio::spawn(run(state, rx, every));

    // Just short of the first interval: still nothing said.
    tokio::time::sleep(every - Duration::from_secs(1)).await;
    assert_eq!(
        lines
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("alive"))
            .count(),
        0,
        "a heartbeat at startup would repeat what the startup lines just said"
    );

    // Past it: exactly one.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        lines
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("alive"))
            .count(),
        1
    );

    // And it keeps going rather than firing once.
    tokio::time::sleep(every).await;
    assert_eq!(
        lines
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("alive"))
            .count(),
        2
    );

    tx.send(true).unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        task.is_finished(),
        "it must stop when shutdown is signalled"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test(start_paused = true)]
async fn the_line_carries_the_numbers_someone_would_look_for() {
    let lines = Lines::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(lines.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let (state, dir) = crate::shell::worker::tests::state_for_update_loop().await;
    state.metrics.accepted();
    state.metrics.delivered(1);

    let (_tx, rx) = watch::channel(false);
    let every = Duration::from_secs(60);
    tokio::spawn(run(Arc::clone(&state), rx, every));
    tokio::time::sleep(every + Duration::from_secs(1)).await;

    let captured = lines.0.lock().unwrap().join("");
    assert!(captured.contains("alive"), "got: {captured}");
    for field in ["accepted=1", "delivered=1", "pending=", "uptime_secs="] {
        assert!(captured.contains(field), "{field} missing from: {captured}");
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_unset_interval_is_an_hour() {
    assert_eq!(interval_from(|_| None), Some(DEFAULT_INTERVAL));
}

#[test]
fn zero_switches_it_off() {
    // A real answer, not a mistake: some machines want their logs
    // untouched.
    assert_eq!(interval_from(|_| Some("0".to_string())), None);
}

#[test]
fn a_number_is_taken_as_seconds() {
    assert_eq!(
        interval_from(|_| Some("90".to_string())),
        Some(Duration::from_secs(90))
    );
    assert_eq!(
        interval_from(|_| Some("  120 ".to_string())),
        Some(Duration::from_secs(120))
    );
}

#[test]
fn a_typo_falls_back_to_the_default_rather_than_to_silence() {
    // The failure mode to avoid: a mistyped value quietly disabling
    // the very thing whose job is to report silence.
    assert_eq!(
        interval_from(|_| Some("hourly".to_string())),
        Some(DEFAULT_INTERVAL)
    );
    assert_eq!(
        interval_from(|_| Some(String::new())),
        Some(DEFAULT_INTERVAL)
    );
}
