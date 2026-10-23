//! Orders playback starts against stops. Resolving an episode takes seconds; a stop or a newer
//! play that arrives meanwhile must win, otherwise the old episode starts playing in the background.

use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::{Mutex, MutexGuard};

#[derive(Default)]
pub struct PlayGate {
    generation: AtomicU64,
    loading: Mutex<()>,
}

impl PlayGate {
    /// Called when a play starts; the ticket goes stale on the next play or stop.
    pub fn begin(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Ticket of the playback running now, for changes within it (another quality).
    pub fn current(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Admits the play to load its file if nothing superseded it. Hold the guard until the
    /// file is loaded: a stop waits for it and then stops that file.
    pub async fn commit(&self, ticket: u64) -> Option<MutexGuard<'_, ()>> {
        let guard = self.loading.lock().await;
        (self.generation.load(Ordering::SeqCst) == ticket).then_some(guard)
    }

    /// Makes every play in flight stale and waits for one that is loading its file right now.
    pub async fn cancel(&self) -> MutexGuard<'_, ()> {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.loading.lock().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn stop_cancels_a_play_that_is_still_loading() {
        let gate = PlayGate::default();
        let ticket = gate.begin();
        drop(gate.cancel().await);
        assert!(gate.commit(ticket).await.is_none());
    }

    #[tokio::test]
    async fn a_change_within_the_playing_episode_is_refused_after_a_stop() {
        let gate = PlayGate::default();
        gate.begin();
        let ticket = gate.current();
        assert!(gate.commit(ticket).await.is_some());
        drop(gate.cancel().await);
        assert!(gate.commit(ticket).await.is_none());
    }

    #[tokio::test]
    async fn a_newer_play_supersedes_an_older_one() {
        let gate = PlayGate::default();
        let older = gate.begin();
        let newer = gate.begin();
        assert!(gate.commit(older).await.is_none());
        assert!(gate.commit(newer).await.is_some());
    }

    #[tokio::test]
    async fn stop_waits_for_a_play_that_is_already_loading_the_file() {
        let gate = Arc::new(PlayGate::default());
        let ticket = gate.begin();
        let committing = gate.commit(ticket).await.expect("latest play commits");
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = tokio::spawn({
            let (gate, stopped) = (gate.clone(), stopped.clone());
            async move {
                let _held = gate.cancel().await;
                stopped.store(true, Ordering::SeqCst);
            }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !stopped.load(Ordering::SeqCst),
            "stop ran while the file was being loaded"
        );
        drop(committing);
        stop.await.unwrap();
        assert!(stopped.load(Ordering::SeqCst));
    }
}
