//! masu's adapter to the new sake (a-kaibu/sake): a game's directory and its .exe in; the game's
//! frames, sounds and console out; the player's input in; all of it through sake's public
//! connection (`sake::session`) and host boundary (`sake::host`) only. masu-lint's
//! sake-session rule keeps this crate off sake's inner tables (windows, handles, threads).
//!
//! A [`Play`] is advanced a turn at a time ([`Play::turn`]) and never blocks on the game's waits:
//! a turn that finds nothing to run returns [`Wake::After`] or [`Wake::Input`], and the caller
//! (masu's server task, a test's loop) sleeps or waits for the player. A finished game is a
//! [`Over`], and a `Play` cannot be advanced after it: [`Play::turn`] and [`Play::stop`] take it.

use std::path::PathBuf;
use std::time::Duration;

use kome::Instant;
use sake::end::End;
use sake::host::native::NativeHost;
use sake::host::{Clock, Console, Control, FileToken, Finished, Frame, HostPath, Input, InputEvent, OpenHow, Opened, Screen, Sound, Speaker, Storage, StorageError, Stream, Ticket, Transfer};
use sake::run::{Options, Reference, Speakers};
use sake::session::{Advanced, Ended, Flow, Pause, Refused, Session, Slice, Started, Wait};

/// A game to start: the directory that is its C:\ and the bytes of its .exe.
#[derive(Clone, Debug)]
pub struct Game {
    /// The directory the game's paths resolve in.
    pub root: PathBuf,
    /// The .exe.
    pub exe: Vec<u8>,
    /// Whether the game sees a sound device (masu's player has one; a check without sound
    /// matches a Windows machine without one).
    pub speakers: Speakers,
}

/// What the game gave out during a turn, in its order within each kind: frames and sounds as
/// sake gave them, then the console's writes of the turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    /// A window's client area, shown.
    Frame(Frame),
    /// A sound stream's change.
    Sound(Sound),
    /// Bytes the game wrote to its console.
    Console {
        /// Which stream.
        stream: Stream,
        /// The bytes.
        bytes: Vec<u8>,
    },
}

/// When to give the game its next turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// Now: the turn used its steps, or its output was taken while the game was held.
    Now,
    /// After this long (the game waits for a timer or a file).
    After(Duration),
    /// When the player gives input (the game waits for nothing else).
    Input,
}

/// A finished game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Over {
    /// How it ended.
    pub end: End,
    /// Instructions run.
    pub steps: u64,
    /// What it gave out in its last turn.
    pub out: Vec<Out>,
}

/// A turn's result.
#[derive(Debug)]
pub enum Turn {
    /// Still playing.
    Playing(Play, Vec<Out>, Wake),
    /// Finished.
    Over(Over),
}

/// A game being played.
#[derive(Debug)]
pub struct Play {
    session: Session<GameHost, Reference>,
}

impl Play {
    /// Starts `game`.
    #[must_use]
    pub fn start(game: Game) -> Turn {
        let host = GameHost { native: NativeHost::new(game.root), console: Vec::new() };
        let options = Options { speakers: game.speakers, ..Options::default() };
        match Session::start(host, Reference, &game.exe, options, Flow::default()) {
            Started::Running(session) => Turn::Playing(Play { session }, Vec::new(), Wake::Now),
            Started::Ended(ended) => Turn::Over(over(ended)),
        }
    }

    /// Runs the game for about `steps` instructions, or until it waits.
    #[must_use]
    pub fn turn(self, steps: u64) -> Turn {
        match self.session.advance(Slice { steps, wait: Wait::Return }) {
            Advanced::Ended(ended) => Turn::Over(over(ended)),
            Advanced::Paused(mut session, pause) => {
                let wake = match pause {
                    Pause::Budget | Pause::Backlog => Wake::Now,
                    Pause::Idle { until: None } => Wake::Input,
                    Pause::Idle { until: Some(at) } => {
                        let now = session.host_mut().now();
                        Wake::After(Duration::from_nanos(at.nanos().saturating_sub(now.nanos())))
                    }
                };
                let mut out: Vec<Out> = std::iter::from_fn(|| session.output()).map(Out::from).collect();
                out.extend(session.host_mut().take_console());
                Turn::Playing(Play { session }, out, wake)
            }
        }
    }

    /// The player's input, arriving now.
    ///
    /// # Errors
    ///
    /// [`Refused::Full`] when the game has not taken the earlier input yet: give it a turn first.
    pub fn press(&mut self, event: InputEvent) -> Result<(), Refused> {
        let now = self.session.host_mut().now();
        self.session.input(now, event)
    }

    /// Stops the game ([`End::Cancelled`]).
    #[must_use]
    pub fn stop(self) -> Over {
        over(self.session.cancel())
    }

    /// Instructions run so far.
    #[must_use]
    pub fn steps(&self) -> u64 {
        self.session.status().steps
    }
}

fn over(ended: Ended<GameHost>) -> Over {
    let Ended { outcome, outputs, mut host } = ended;
    let mut out: Vec<Out> = outputs.into_iter().map(Out::from).collect();
    out.extend(host.take_console());
    Over { end: outcome.end, steps: outcome.steps, out }
}

impl From<sake::host::Output> for Out {
    fn from(output: sake::host::Output) -> Out {
        match output {
            sake::host::Output::Frame(frame) => Out::Frame(frame),
            sake::host::Output::Sound(sound) => Out::Sound(sound),
        }
    }
}

/// The host a game runs on: the game's directory and the real clock (sake's `NativeHost`), with
/// the console kept for masu instead of written to masu's own stdout. Frames and sounds reach
/// masu as the session's outputs, so the host's own screen and speaker are never given any.
#[derive(Debug)]
struct GameHost {
    native: NativeHost,
    console: Vec<Out>,
}

impl GameHost {
    fn take_console(&mut self) -> Vec<Out> {
        core::mem::take(&mut self.console)
    }
}

impl Storage for GameHost {
    fn open(&mut self, path: &HostPath, how: OpenHow) -> Result<Opened, StorageError> {
        self.native.open(path, how)
    }

    fn close(&mut self, file: FileToken) {
        self.native.close(file);
    }

    fn submit(&mut self, ticket: Ticket, transfer: Transfer, now: Instant) {
        self.native.submit(ticket, transfer, now);
    }

    fn cancel(&mut self, io: kome::IoId, now: Instant) {
        self.native.cancel(io, now);
    }

    fn poll(&mut self, now: Instant) -> Option<Finished> {
        self.native.poll(now)
    }

    fn next_ready(&mut self) -> Option<Instant> {
        self.native.next_ready()
    }
}

impl Clock for GameHost {
    fn now(&mut self) -> Instant {
        self.native.now()
    }

    fn sleep_until(&mut self, at: Instant) {
        self.native.sleep_until(at);
    }

    fn ran(&mut self, steps: u64) {
        self.native.ran(steps);
    }
}

impl Console for GameHost {
    fn write(&mut self, stream: Stream, bytes: &[u8]) -> Result<(), StorageError> {
        match self.console.last_mut() {
            Some(Out::Console { stream: last, bytes: kept }) if *last == stream => kept.extend_from_slice(bytes),
            Some(Out::Console { .. } | Out::Frame(_) | Out::Sound(_)) | None => self.console.push(Out::Console { stream, bytes: bytes.to_vec() }),
        }
        Ok(())
    }
}

impl Control for GameHost {
    fn halted(&self) -> bool {
        self.native.halted()
    }
}

impl Speaker for GameHost {
    // (a session holds its sounds for Play::turn to take; none is given to the host)
    fn sound(&mut self, sound: Sound) {
        self.native.sound(sound);
    }
}

impl Screen for GameHost {
    // (a session holds its frames for Play::turn to take; none is given to the host)
    fn frame(&mut self, frame: Frame) {
        self.native.frame(frame);
    }
}

impl Input for GameHost {
    // (the player's input reaches the game through Play::press, the session's input)
    fn input(&mut self, now: Instant) -> Option<InputEvent> {
        self.native.input(now)
    }

    fn next_input(&mut self) -> Option<Instant> {
        self.native.next_input()
    }
}
