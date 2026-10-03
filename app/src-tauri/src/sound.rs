//! The alarm's looping sound on Linux.
//!
//! The choice: a short alarm tone bundled in the app (`assets/alarm.wav`,
//! generated, public domain), written to the runtime directory on first use
//! and played over and over by whichever command-line player the desktop has,
//! tried in order: `pw-play` (PipeWire), `paplay` (PulseAudio, also served by
//! PipeWire), `aplay` (ALSA), `ffplay`, `canberra-gtk-play`. Stopping kills
//! the player.
//!
//! Why not a Rust audio crate or libcanberra: `cpal`/`rodio` need the ALSA
//! development headers at build time and open the sound card directly,
//! bypassing PipeWire's routing on some setups; libcanberra plays event
//! sounds from the theme once, not a loop, and isn't installed everywhere.
//! A player process adds no build dependency, goes through the desktop's own
//! sound server, and dies with a `kill`. The cost is that a desktop with none
//! of the players has no alarm sound (the critical notification and window
//! still appear); that is logged once per alarm.
//!
//! [`Sound`] is the seam the tests fake; [`LoopingSound`] is the real one,
//! and its player commands can be replaced so it is tested without audio.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The bundled alarm tone.
const ALARM_WAV: &[u8] = include_bytes!("../assets/alarm.wav");

/// What the app asks of the alarm sound.
pub trait Sound: Send {
    /// Starts looping, if it isn't already.
    fn start(&mut self);
    /// Stops at once, if it is looping.
    fn stop(&mut self);
}

/// A command that plays a file; `{file}` in an argument is replaced by its
/// path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    pub program: String,
    pub args: Vec<String>,
}

impl Player {
    pub fn new(program: &str, args: &[&str]) -> Self {
        Player {
            program: program.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }

    fn spawn(&self, file: &Path) -> std::io::Result<Child> {
        let file = file.to_string_lossy();
        Command::new(&self.program)
            .args(self.args.iter().map(|a| a.replace("{file}", &file)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }
}

/// The players to try, best first.
pub fn default_players() -> Vec<Player> {
    vec![
        Player::new("pw-play", &["{file}"]),
        Player::new("paplay", &["{file}"]),
        Player::new("aplay", &["-q", "{file}"]),
        Player::new(
            "ffplay",
            &["-nodisp", "-autoexit", "-loglevel", "quiet", "{file}"],
        ),
        Player::new("canberra-gtk-play", &["-f", "{file}"]),
    ]
}

/// A player that exits with a failure sooner than this didn't really play.
const FAILED_FAST: Duration = Duration::from_secs(1);
/// How often the playing thread looks at the stop flag.
const POLL: Duration = Duration::from_millis(40);

struct Playing {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

/// Loops the alarm tone through a command-line player.
pub struct LoopingSound {
    players: Vec<Player>,
    file: PathBuf,
    playing: Option<Playing>,
}

impl LoopingSound {
    /// The real alarm: the bundled tone, written under the runtime directory.
    pub fn bundled() -> Self {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let file = dir.join("hab-bot-alarm.wav");
        if let Err(e) = std::fs::write(&file, ALARM_WAV) {
            eprintln!("cannot write the alarm sound to {}: {e}", file.display());
        }
        Self::with_players(default_players(), file)
    }

    pub fn with_players(players: Vec<Player>, file: PathBuf) -> Self {
        LoopingSound {
            players,
            file,
            playing: None,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
            .as_ref()
            .is_some_and(|p| !p.thread.is_finished())
    }
}

impl Sound for LoopingSound {
    fn start(&mut self) {
        if self.is_playing() {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (players, file, flag) = (self.players.clone(), self.file.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("alarm-sound".into())
            .spawn(move || play_until_stopped(&players, &file, &flag));
        match thread {
            Ok(thread) => self.playing = Some(Playing { stop, thread }),
            Err(e) => eprintln!("cannot start the alarm sound: {e}"),
        }
    }

    fn stop(&mut self) {
        if let Some(p) = self.playing.take() {
            p.stop.store(true, Ordering::SeqCst);
            let _ = p.thread.join();
        }
    }
}

impl Drop for LoopingSound {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Plays with the first player that works, again and again, until `stop`.
fn play_until_stopped(players: &[Player], file: &Path, stop: &AtomicBool) {
    let mut index = 0;
    while !stop.load(Ordering::SeqCst) {
        let Some(player) = players.get(index) else {
            eprintln!("no player for the alarm sound (tried pw-play, paplay, aplay, ffplay, canberra-gtk-play)");
            return;
        };
        let mut child = match player.spawn(file) {
            Ok(c) => c,
            Err(_) => {
                index += 1;
                continue;
            }
        };
        let started = Instant::now();
        let status = loop {
            if stop.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => std::thread::sleep(POLL),
                Err(_) => break None,
            }
        };
        let worked = status.is_some_and(|s| s.success()) || started.elapsed() >= FAILED_FAST;
        if !worked {
            index += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hab-sound-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn lines(path: &Path) -> usize {
        std::fs::read_to_string(path).map_or(0, |s| s.lines().count())
    }

    fn wait_for(mut f: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// A "player" that logs that it played `{file}` then plays for `secs`.
    fn logging_player(log: &Path, secs: &str) -> Player {
        Player::new(
            "sh",
            &[
                "-c",
                &format!("echo \"$0\" >> {}; sleep {secs}", log.display()),
                "{file}",
            ],
        )
    }

    #[test]
    fn the_bundled_tone_is_a_wav_file() {
        assert_eq!(&ALARM_WAV[..4], b"RIFF");
        assert_eq!(&ALARM_WAV[8..12], b"WAVE");
    }

    #[test]
    fn it_plays_the_file_until_stopped_and_stops_at_once() {
        let dir = temp("play");
        let log = dir.join("log");
        let mut s =
            LoopingSound::with_players(vec![logging_player(&log, "30")], dir.join("alarm.wav"));
        s.start();
        assert!(wait_for(|| lines(&log) == 1));
        assert!(s.is_playing());
        // Starting again doesn't start a second player.
        s.start();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(lines(&log), 1);
        assert_eq!(
            std::fs::read_to_string(&log).unwrap().trim(),
            dir.join("alarm.wav").to_string_lossy()
        );
        let before = Instant::now();
        s.stop();
        assert!(
            before.elapsed() < Duration::from_secs(2),
            "the player was killed"
        );
        assert!(!s.is_playing());
        // It can ring again later.
        s.start();
        assert!(wait_for(|| lines(&log) == 2));
        s.stop();
    }

    #[test]
    fn it_loops_by_playing_again_when_the_player_finishes() {
        let dir = temp("loop");
        let log = dir.join("log");
        let mut s =
            LoopingSound::with_players(vec![logging_player(&log, "0.1")], dir.join("alarm.wav"));
        s.start();
        assert!(wait_for(|| lines(&log) >= 3), "plays over and over");
        s.stop();
        let after = lines(&log);
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(lines(&log), after, "nothing plays once stopped");
    }

    #[test]
    fn it_falls_back_past_a_missing_or_failing_player() {
        let dir = temp("fallback");
        let log = dir.join("log");
        let mut s = LoopingSound::with_players(
            vec![
                Player::new("definitely-not-a-player", &["{file}"]),
                Player::new("sh", &["-c", "exit 1"]),
                logging_player(&log, "30"),
            ],
            dir.join("alarm.wav"),
        );
        s.start();
        assert!(wait_for(|| lines(&log) == 1));
        s.stop();
    }

    #[test]
    fn with_no_working_player_it_gives_up_quietly() {
        let dir = temp("none");
        let mut s = LoopingSound::with_players(
            vec![Player::new("definitely-not-a-player", &["{file}"])],
            dir.join("alarm.wav"),
        );
        s.start();
        assert!(wait_for(|| !s.is_playing()));
        s.stop();
    }
}
