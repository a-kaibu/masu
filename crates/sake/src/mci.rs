//! winmm's MCI (mciSendCommand / mciSendString / mciGetErrorString / mciGetDeviceID) and
//! msvfw32's MCIWnd, for the devices games play sound and movies with:
//!
//! - `waveaudio` (.wav) and `mpegvideo` (MP3 / MPEG / AVI / WMV, a file with no extension opened
//!   with `type mpegvideo`): a sound plays through the mixer (sound.rs) as PCM, MP3 and the
//!   others decoded by the page's AudioDecoder (audiofile.rs, kura::adec); a movie plays from
//!   the `.vnv` `masu pack` made next to it (dshow_play.rs), in the window the program gives
//!   (MCI_WINDOW / MCI_DGV_OPEN_PARENT / MCIWnd), else on the screen.
//! - open (type, element, alias, by extension), close (one or all), play (from, to, notify,
//!   wait, repeat), stop, pause, resume, seek (to, start, end), status (length, position, mode,
//!   ready, media present, time format, tracks), set (time format: milliseconds; bytes and
//!   samples for waveaudio; frames for a movie), setaudio volume, window, put / where
//!   destination, capability, info.
//! - Notifications as Windows sends them (MM_MCINOTIFY to the callback window): a command with
//!   MCI_NOTIFY reports when it is done; a play's report waits for the end (SUCCESSFUL), and
//!   another command with notify supersedes it (SUPERSEDED), a stop, seek or close aborts it
//!   (ABORTED). See `Notes`.
//! - MCIWnd ("MCIWndClass"): MCIWNDM_OPEN, MCI_PLAY / STOP / PAUSE / RESUME / SEEK / CLOSE,
//!   MCIWNDM_PLAYFROM / PLAYTO / GETMODE / GETLENGTH / GETPOSITION / GETSTART / GETEND /
//!   CAN_PLAY / CAN_WINDOW / GETDEVICEID / SETVOLUME / GETVOLUME / SETREPEAT, and
//!   MCIWNDM_NOTIFYMODE to the parent (MCIWNDF_NOTIFYMODE) as the mode changes; the window
//!   takes the movie's size unless MCIWNDF_NOAUTOSIZEWINDOW. The play bar and menu are not
//!   drawn.
//!
//! Not here: `sequencer` (MIDI) and `cdaudio` (McErr::CannotLoadDriver), recording, and the
//! less used commands (McErr::UnsupportedFunction).

#![allow(non_snake_case)]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::api::{Ctx, Handler};
use crate::dshow_play::{self as play, Timeline, SEC};
use crate::mem::{self, r32, w32};

// ---------------------------------------------------------------------------------------------
// constants (mmsystem.h, digitalv.h, vfw.h)


/// MCIERR_* (mmsystem.h): what an MCI call fails with, by name; `code` is the number the
/// program sees.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum McErr {
    InvalidDeviceId,
    UnrecognizedKeyword,
    UnrecognizedCommand,
    InvalidDeviceName,
    CannotLoadDriver,
    MissingStringArgument,
    BadInteger,
    MissingParameter,
    UnsupportedFunction,
    FileNotFound,
    OutOfRange,
    ExtensionNotFound,
    DuplicateAlias,
    BadTimeFormat,
    InvalidFile,
    NullParameterBlock,
    NoInteger,
}
impl McErr {
    const ALL: [McErr; 17] = [McErr::InvalidDeviceId, McErr::UnrecognizedKeyword, McErr::UnrecognizedCommand, McErr::InvalidDeviceName, McErr::CannotLoadDriver, McErr::MissingStringArgument, McErr::BadInteger, McErr::MissingParameter, McErr::UnsupportedFunction, McErr::FileNotFound, McErr::OutOfRange, McErr::ExtensionNotFound, McErr::DuplicateAlias, McErr::BadTimeFormat, McErr::InvalidFile, McErr::NullParameterBlock, McErr::NoInteger];
    pub fn code(self) -> u32 {
        match self {
            McErr::InvalidDeviceId => 257,
            McErr::UnrecognizedKeyword => 259,
            McErr::UnrecognizedCommand => 261,
            McErr::InvalidDeviceName => 263,
            McErr::CannotLoadDriver => 266,
            McErr::MissingStringArgument => 269,
            McErr::BadInteger => 270,
            McErr::MissingParameter => 273,
            McErr::UnsupportedFunction => 274,
            McErr::FileNotFound => 275,
            McErr::OutOfRange => 282,
            McErr::ExtensionNotFound => 281,
            McErr::DuplicateAlias => 289,
            McErr::BadTimeFormat => 293,
            McErr::InvalidFile => 296,
            McErr::NullParameterBlock => 297,
            McErr::NoInteger => 312,
        }
    }
    /// The error a program's number names (None: 0 or one MCI here does not give)
    pub fn from_code(e: u32) -> Option<McErr> {
        McErr::ALL.into_iter().find(|x| x.code() == e)
    }
}

const MCI_OPEN: u32 = 0x803;
const MCI_CLOSE: u32 = 0x804;
const MCI_PLAY: u32 = 0x806;
const MCI_SEEK: u32 = 0x807;
const MCI_STOP: u32 = 0x808;
const MCI_PAUSE: u32 = 0x809;
const MCI_INFO: u32 = 0x80a;
const MCI_GETDEVCAPS: u32 = 0x80b;
const MCI_SET: u32 = 0x80d;
const MCI_STATUS: u32 = 0x814;
const MCI_CUE: u32 = 0x830;
const MCI_WINDOW: u32 = 0x841;
const MCI_PUT: u32 = 0x842;
const MCI_WHERE: u32 = 0x843;
const MCI_RESUME: u32 = 0x855;
const MCI_SETAUDIO: u32 = 0x873;

const MCI_NOTIFY: u32 = 1;
const MCI_WAIT: u32 = 2;
const MCI_FROM: u32 = 4;
const MCI_TO: u32 = 8;

const MCI_OPEN_ELEMENT: u32 = 0x200;
const MCI_OPEN_ALIAS: u32 = 0x400;
const MCI_OPEN_TYPE_ID: u32 = 0x1000;
const MCI_OPEN_TYPE: u32 = 0x2000;
const MCI_DGV_OPEN_WS: u32 = 0x1_0000;
const MCI_DGV_OPEN_PARENT: u32 = 0x2_0000;
const MCI_DGV_PLAY_REPEAT: u32 = 0x1_0000;
const MCI_SEEK_TO_START: u32 = 0x100;
const MCI_SEEK_TO_END: u32 = 0x200;
const MCI_STATUS_ITEM: u32 = 0x100;
const MCI_SET_TIME_FORMAT: u32 = 0x400;
const MCI_GETDEVCAPS_ITEM: u32 = 0x100;
const MCI_DGV_WINDOW_HWND: u32 = 0x1_0000;
const MCI_DGV_RECT: u32 = 0x1_0000;
const MCI_DGV_PUT_DESTINATION: u32 = 0x4_0000;
const MCI_DGV_WHERE_DESTINATION: u32 = 0x4_0000;
const MCI_DGV_SETAUDIO_ITEM: u32 = 0x80_0000;
const MCI_DGV_SETAUDIO_VALUE: u32 = 0x100_0000;
const MCI_DGV_SETAUDIO_VOLUME: u32 = 0x4002;

const MCI_STATUS_LENGTH: u32 = 1;
const MCI_STATUS_POSITION: u32 = 2;
const MCI_STATUS_NUMBER_OF_TRACKS: u32 = 3;
const MCI_STATUS_MODE: u32 = 4;
const MCI_STATUS_MEDIA_PRESENT: u32 = 5;
const MCI_STATUS_TIME_FORMAT: u32 = 6;
const MCI_STATUS_READY: u32 = 7;
const MCI_STATUS_CURRENT_TRACK: u32 = 8;
const MCI_DGV_STATUS_VOLUME: u32 = 0x4019;

/// MCI_MODE_* a device is in
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    NotReady,
    Stop,
    Play,
    Pause,
}
impl Mode {
    pub fn code(self) -> u32 {
        match self {
            Mode::NotReady => 524,
            Mode::Stop => 525,
            Mode::Play => 526,
            Mode::Pause => 529,
        }
    }
    /// Its name in a status string and MCIWNDM_GETMODE
    fn name(self) -> &'static str {
        match self {
            Mode::NotReady => "not ready",
            Mode::Stop => "stopped",
            Mode::Play => "playing",
            Mode::Pause => "paused",
        }
    }
}

/// MCI_FORMAT_* a device here takes (MCI_SET_TIME_FORMAT)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimeFormat {
    Ms,
    Frames,
    Bytes,
    Samples,
}
impl TimeFormat {
    pub fn code(self) -> u32 {
        match self {
            TimeFormat::Ms => 0,
            TimeFormat::Frames => 3,
            TimeFormat::Bytes => 8,
            TimeFormat::Samples => 9,
        }
    }
    /// None: SMPTE, HMS, TMSF ... (none taken here)
    pub fn from_code(f: u32) -> Option<TimeFormat> {
        [TimeFormat::Ms, TimeFormat::Frames, TimeFormat::Bytes, TimeFormat::Samples].into_iter().find(|x| x.code() == f)
    }
}

const MCI_ALL_DEVICE_ID: u32 = 0xffff;
const MM_MCINOTIFY: u32 = 0x3b9;
pub const MCI_NOTIFY_SUCCESSFUL: u32 = 1;
pub const MCI_NOTIFY_SUPERSEDED: u32 = 2;
pub const MCI_NOTIFY_ABORTED: u32 = 4;
pub const MCI_NOTIFY_FAILURE: u32 = 8;

// devices (MCI_DEVTYPE_*)
const DEVTYPE_WAVEFORM_AUDIO: u32 = 522;
const DEVTYPE_DIGITAL_VIDEO: u32 = 520;
const DEVTYPE_SEQUENCER: u32 = 523;
const DEVTYPE_CD_AUDIO: u32 = 516;

// ---------------------------------------------------------------------------------------------
// the pure parts

/// The kinds of device MCI opens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Wave,
    Mpeg,
    Avi,
}

/// A device type by its name (`type x`, MCI_OPEN_TYPE): Some(Err) for one Windows has but not
/// here (MIDI, CD), None for one it doesn't know.
pub fn kind_of_type(t: &str) -> Option<Result<Kind, McErr>> {
    match t.to_ascii_lowercase().as_str() {
        "waveaudio" => Some(Ok(Kind::Wave)),
        "mpegvideo" | "mpegvideo2" | "digitalvideo" => Some(Ok(Kind::Mpeg)),
        "avivideo" => Some(Ok(Kind::Avi)),
        "sequencer" | "cdaudio" | "videodisc" | "vcr" | "overlay" | "scanner" | "animation" | "dat" => Some(Err(McErr::CannotLoadDriver)),
        _ => None,
    }
}

/// A device type by MCI_OPEN_TYPE_ID's low word (MCI_DEVTYPE_*).
fn kind_of_id(id: u32) -> Result<Kind, McErr> {
    match id & 0xffff {
        DEVTYPE_WAVEFORM_AUDIO => Ok(Kind::Wave),
        DEVTYPE_DIGITAL_VIDEO => Ok(Kind::Mpeg),
        DEVTYPE_SEQUENCER | DEVTYPE_CD_AUDIO => Err(McErr::CannotLoadDriver),
        _ => Err(McErr::InvalidDeviceName),
    }
}

/// The device a file opens by its extension (the registry's [MCI Extensions]): None for one
/// with no extension or one not listed.
pub fn kind_of_file(path: &str) -> Option<Result<Kind, McErr>> {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let (_, ext) = name.rsplit_once('.')?;
    Some(match ext.to_ascii_lowercase().as_str() {
        "wav" => Ok(Kind::Wave),
        "avi" => Ok(Kind::Avi),
        "mp3" | "mp2" | "mpa" | "mpg" | "mpeg" | "m1v" | "m2v" | "mpe" | "mpv2" | "m1a" | "m2a" | "wma" | "wmv" | "asf" | "aif" | "aiff" | "aifc" | "au" | "snd" | "m4a" | "mp4" | "mov" => Ok(Kind::Mpeg),
        "mid" | "midi" | "rmi" | "cda" => Err(McErr::CannotLoadDriver),
        _ => return None,
    })
}

/// A device's position or length (100 ns) in its time format: ms, frames of the movie's rate
/// (num/den a second), bytes or samples of the sound's format (rate, block).
pub fn to_format(t: u64, format: TimeFormat, rate: u32, block: u32, fps: (u32, u32)) -> u32 {
    let (t, sec) = (u128::from(t), u128::from(SEC));
    let v = match format {
        TimeFormat::Frames if fps.0 != 0 => t * u128::from(fps.0) / (u128::from(fps.1.max(1)) * sec),
        TimeFormat::Samples => t * u128::from(rate) / sec,
        TimeFormat::Bytes => t * u128::from(rate) / sec * u128::from(block),
        // (a movie with no frame rate: in ms)
        TimeFormat::Ms | TimeFormat::Frames => (t + 5_000) / 10_000,
    };
    u32::try_from(v).unwrap_or(u32::MAX)
}

/// A value in the time format back to 100 ns.
pub fn from_format(v: u32, format: TimeFormat, rate: u32, block: u32, fps: (u32, u32)) -> u64 {
    let (v, sec) = (u128::from(v), u128::from(SEC));
    let t = match format {
        // (rounded up: the time of a frame or sample is where it starts, which reads back as it)
        TimeFormat::Frames if fps.0 != 0 => (v * u128::from(fps.1.max(1)) * sec).div_ceil(u128::from(fps.0)),
        TimeFormat::Samples if rate != 0 => (v * sec).div_ceil(u128::from(rate)),
        TimeFormat::Bytes if rate != 0 && block != 0 => (v / u128::from(block) * sec).div_ceil(u128::from(rate)),
        // (no rate or block to count in: as ms)
        TimeFormat::Ms | TimeFormat::Frames | TimeFormat::Samples | TimeFormat::Bytes => v * 10_000,
    };
    u64::try_from(t).unwrap_or(u64::MAX)
}

/// Whether a device of `kind` (a movie when `video`) takes the time format.
pub fn format_ok(kind: Kind, video: bool, format: TimeFormat) -> bool {
    match format {
        TimeFormat::Ms => true,
        TimeFormat::Bytes | TimeFormat::Samples => kind == Kind::Wave,
        TimeFormat::Frames => kind != Kind::Wave && video,
    }
}

/// MCI's volume (0..1000, setaudio) as hundredths of a dB (DirectSound's), and back.
pub fn volume_db(v: u32) -> i32 {
    if v == 0 {
        return -10000;
    }
    ((2000.0 * (v.min(1000) as f64 / 1000.0).log10()).round() as i32).clamp(-10000, 0)
}
pub fn volume_mci(db: i32) -> u32 {
    if db <= -10000 {
        return 0;
    }
    (1000.0 * 10f64.powf(db.min(0) as f64 / 2000.0)).round() as u32
}

/// The notifications a device owes: the one a play waits to give at its end, if any. Each
/// change returns what goes out now: (window, MCI_NOTIFY_*).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notes {
    pub pending: Option<u32>,
}

impl Notes {
    /// A command arrives (`notify`: with MCI_NOTIFY to `hwnd`); `interrupts`: it stops a play
    /// (stop, seek, close, a new play). A pending report is superseded by a new notify and
    /// aborted by an interruption without one.
    pub fn command(&mut self, notify: Option<u32>, interrupts: bool) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        if let Some(w) = self.pending {
            if notify.is_some() {
                out.push((w, MCI_NOTIFY_SUPERSEDED));
                self.pending = None;
            } else if interrupts {
                out.push((w, MCI_NOTIFY_ABORTED));
                self.pending = None;
            }
        }
        out
    }
    /// A play with notify starts waiting for its end.
    pub fn wait_end(&mut self, hwnd: u32) {
        self.pending = Some(hwnd);
    }
    /// The play reached its end.
    pub fn end(&mut self) -> Vec<(u32, u32)> {
        self.pending.take().map(|w| vec![(w, MCI_NOTIFY_SUCCESSFUL)]).unwrap_or_default()
    }
}

/// A command string's words: spaces separate them, double quotes keep spaces in one (and are
/// dropped). None for an unclosed quote.
pub fn words(s: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for ch in s.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if any {
        out.push(cur);
    }
    Some(out)
}

/// Windows' text for an MCI error (en-US), None for a code it has none for.
pub fn error_text(e: u32) -> Option<&'static str> {
    if e == 0 {
        return Some("The specified command was carried out.");
    }
    Some(match McErr::from_code(e)? {
        McErr::InvalidDeviceId => "Invalid MCI device ID. Use the ID returned when opening the MCI device.",
        McErr::UnrecognizedKeyword => "The driver cannot recognize the specified command parameter.",
        McErr::UnrecognizedCommand => "The driver cannot recognize the specified command.",
        McErr::InvalidDeviceName => "The specified device is not open or is not recognized by MCI.",
        McErr::CannotLoadDriver => "The specified device driver will not load properly.",
        McErr::MissingStringArgument => "The specified command requires a character-string parameter. Please provide one.",
        McErr::BadInteger => "The specified integer is invalid for this command.",
        McErr::MissingParameter => "The specified command requires a parameter. Please supply one.",
        McErr::UnsupportedFunction => "The MCI device you are using does not support the specified command.",
        McErr::FileNotFound => "Cannot find the specified file. Make sure the path and filename are correct.",
        McErr::ExtensionNotFound => "The specified extension has no device type associated with it. Specify a device when performing this command.",
        McErr::OutOfRange => "The specified parameter is out of range for the specified command.",
        McErr::DuplicateAlias => "The specified alias is already being used in this application. Use a unique alias.",
        McErr::BadTimeFormat => "The specified format is invalid for this command.",
        McErr::InvalidFile => "The specified file cannot be played on the specified MCI device. The file may be corrupt, not in the correct format, or no file handler available for this format.",
        McErr::NullParameterBlock => "A null parameter block was passed to MCI.",
        McErr::NoInteger => "The parameter for this MCI command must be an integer value.",
    })
}

// ---------------------------------------------------------------------------------------------
// devices

enum Media {
    /// a sound in guest memory the mixer plays under `key`
    Sound { data: u32, size: u32, fmt: crate::sound::Fmt },
    /// a movie (dshow_play) under `key`
    Movie { fps: (u32, u32), size: (u32, u32) },
    /// a sound with nothing to play it from (not decodable here): only its timing
    Silent { rate: u32, block: u32 },
}

struct Dev {
    kind: Kind,
    alias: String,
    element: String,
    media: Media,
    /// the play key of the mixer / the movie thread (a guest block of its own)
    key: u32,
    /// timing of a sound (a movie's is in its Play)
    tl: Timeline,
    duration: u64,
    format: TimeFormat,
    notes: Notes,
    /// `repeat`: from the start again at the end
    repeat: bool,
    /// bumped by every command that changes when the end comes (the waiter's stale check)
    gen: u32,
    /// the movie's window (MCI_WINDOW, the open's parent, an MCIWnd) and its place in it
    hwnd: u32,
    dst: Option<(i32, i32, i32, i32)>,
    volume: i32,
    /// the MCIWnd showing it (MCIWNDM_NOTIFYMODE as the mode changes)
    mciwnd: u32,
}

impl Dev {
    fn video(&self) -> bool {
        matches!(self.media, Media::Movie { .. })
    }
    fn rate_block(&self) -> (u32, u32) {
        match &self.media {
            Media::Sound { fmt, .. } => (fmt.rate, fmt.block()),
            Media::Silent { rate, block } => (*rate, *block),
            Media::Movie { .. } => (0, 0),
        }
    }
    fn fps(&self) -> (u32, u32) {
        match self.media {
            Media::Movie { fps, .. } => fps,
            Media::Sound { .. } | Media::Silent { .. } => (0, 1),
        }
    }
    fn to_fmt(&self, t: u64) -> u32 {
        let (r, b) = self.rate_block();
        to_format(t, self.format, r, b, self.fps())
    }
    fn from_fmt(&self, v: u32) -> u64 {
        let (r, b) = self.rate_block();
        from_format(v, self.format, r, b, self.fps()).min(self.duration)
    }
}

#[derive(Default)]
struct State {
    devs: HashMap<u32, Dev>,
    next: u32,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| State { devs: HashMap::new(), next: 1 }))
}
fn dev<R>(id: u32, f: impl FnOnce(&mut Dev) -> R) -> Option<R> {
    with(|s| s.devs.get_mut(&id).map(f))
}

/// The timeline of device `id` (the movie's own for a movie) to change, with the time now.
fn timeline<R>(id: u32, f: impl FnOnce(&mut Timeline, u64) -> R) -> Option<R> {
    let (key, video) = dev(id, |d| (d.key, d.video()))?;
    let t = play::now();
    if video {
        play::timeline(key, t, f)
    } else {
        dev(id, |d| f(&mut d.tl, t))
    }
}

/// playing (running, short of its stop), paused (held mid-play), else stopped
fn mode(id: u32) -> Mode {
    let Some(playing) = timeline(id, |tl, t| tl.running() && tl.position(t) < tl.stop) else { return Mode::NotReady };
    if playing {
        Mode::Play
    } else if dev(id, |d| d.paused()).unwrap_or(false) {
        Mode::Pause
    } else {
        Mode::Stop
    }
}

impl Dev {
    /// paused (not stopped): a play is under way but held
    fn paused(&self) -> bool {
        self.gen & 0x8000_0000 != 0
    }
}

fn post_all(out: Vec<(u32, u32)>, id: u32) {
    for (w, code) in out {
        if w != 0 {
            crate::user::post(w, MM_MCINOTIFY, code, id);
        }
    }
}

/// The sound's mixer state follows its timeline (where it is, whether it plays).
fn sync_sound(id: u32) {
    let Some((key, frames_at, playing)) = dev(id, |d| {
        let t = play::now();
        let pos = d.tl.position(t);
        let rate = d.rate_block().0 as f64;
        (d.key, pos as f64 * rate / SEC as f64, d.tl.running() && pos < d.tl.stop)
    }) else {
        return;
    };
    crate::sound::music_set(key, Some(frames_at), Some(playing));
}

/// A device's movie gets its place (window, rectangle) and volume.
fn sync_movie(id: u32) {
    let Some((key, hwnd, dst, size, volume)) = dev(id, |d| (d.key, d.hwnd, d.dst, match d.media { Media::Movie { size, .. } => size, _ => (0, 0) }, d.volume)) else { return };
    let client = if hwnd != 0 { crate::user::client_rect_screen(hwnd).map(|r| (r.r - r.l, r.b - r.t)) } else { None };
    let dst = dst.unwrap_or_else(|| match client {
        // (an MCIWnd and a window given: its client area; the screen: the movie's size)
        Some((w, h)) => (0, 0, w, h),
        None => (0, 0, size.0 as i32, size.1 as i32),
    });
    play::with(|m| {
        if let Some(p) = m.get_mut(&key) {
            p.place = play::Place { hwnd, dst, src: (0, 0, size.0 as i32, size.1 as i32), letterbox: false, visible: true };
            p.volume = volume;
        }
    });
}

/// Arms the wait for device `id`'s end: on the pool's timer thread, which (if nothing changed
/// the device meanwhile) reports it (notify, repeat, MCIWnd's mode). Each play used to start a
/// guest thread of its own (and leave its handle open).
fn arm(id: u32) {
    let Some(gen) = dev(id, |d| {
        d.gen = (d.gen + 1) & 0x7fff_ffff;
        d.gen
    }) else {
        return;
    };
    end_check(id, gen);
}

/// Bumps the generation (a wait in flight gives up), keeping the paused flag.
fn disarm(id: u32) {
    dev(id, |d| d.gen = (d.gen & 0x8000_0000) | ((d.gen + 1) & 0x7fff_ffff));
    crate::pool::TIMERS.cancel(crate::pool::key::MCI | id as u64);
}

/// Device `id` under generation `gen`: its end now, or a look again when it is due (in 250 ms
/// at most: the timeline may change under it).
fn end_check(id: u32, gen: u32) {
    loop {
        let live = dev(id, |d| d.gen & 0x7fff_ffff == gen && !d.paused()).unwrap_or(false);
        if !live {
            return;
        }
        let Some(left) = timeline(id, |tl, t| tl.until_end(t)).flatten() else { return };
        if left > 0 {
            let d = Duration::from_nanos(left * 100).min(Duration::from_millis(250));
            crate::pool::TIMERS.after(crate::pool::key::MCI | id as u64, d, Box::new(move |_| end_check(id, gen)));
            return;
        }
        // the end: once (a movie's may come from the movie thread first: `ended`)
        if !timeline(id, |tl, t| tl.complete(t)).unwrap_or(false) {
            return;
        }
        if !finished(id) {
            return;
        }
    }
}

/// Device `id` played to its end: again from the start (repeat; true), or its report goes out
/// and an MCIWnd shows the new mode.
fn finished(id: u32) -> bool {
    let (repeat, mciwnd) = dev(id, |d| (d.repeat, d.mciwnd)).unwrap_or((false, 0));
    if repeat {
        timeline(id, |tl, t| {
            tl.seek(t, 0);
            tl.run(t);
        });
        sync_sound(id);
        return true;
    }
    let out = dev(id, |d| d.notes.end()).unwrap_or_default();
    post_all(out, id);
    sync_sound(id);
    if mciwnd != 0 {
        crate::user::post(mciwnd, WM_MCIWND_MODE, 0, 0);
    }
    false
}

/// The movie thread saw the movie under `key` end: whether it is one of ours.
pub fn ended(key: u32) -> bool {
    let Some(id) = with(|s| s.devs.iter().find(|(_, d)| d.key == key && d.video()).map(|(&i, _)| i)) else { return false };
    if finished(id) {
        // (repeating: its waiter again)
        arm(id);
    }
    true
}

/// Opens device `kind` (None: by the element's extension) on `element`, as `alias`.
fn open(kind: Option<Kind>, element: &str, alias: Option<&str>) -> Result<u32, McErr> {
    if element.is_empty() {
        // (a device opened without a file: only a simple device; these are compound)
        return Err(McErr::UnsupportedFunction);
    }
    let kind = match kind {
        Some(k) => k,
        None => kind_of_file(element).ok_or(McErr::ExtensionNotFound)??,
    };
    let alias = alias.map(|a| a.to_ascii_lowercase()).unwrap_or_else(|| element.to_ascii_lowercase());
    if with(|s| s.devs.values().any(|d| d.alias == alias)) {
        return Err(McErr::DuplicateAlias);
    }
    let full = crate::files::full_path(element);
    let vnv = play::Movie::find(&full);
    let key = mem::heap_alloc(4, true);
    let (media, duration) = if let Some(movie) = vnv.filter(|_| kind != Kind::Wave) {
        let m = Media::Movie { fps: movie.rate, size: (movie.w, movie.h) };
        let d = movie.duration;
        eprintln!("sake: movie {element}: {}x{} {} frames (MCI)", movie.w, movie.h, movie.frames);
        let mut p = play::Play::new(movie);
        p.place.visible = false;
        play::start(key, p);
        (m, d)
    } else {
        let Some(host) = crate::files::resolve(&full) else {
            mem::heap_free(key);
            return Err(McErr::FileNotFound);
        };
        // (a movie with no .vnv is not read whole to find it is no sound file)
        let Some(head) = crate::audiofile::head_of(&host) else {
            mem::heap_free(key);
            return Err(McErr::FileNotFound);
        };
        let bytes = if crate::audiofile::maybe_sound(&head) { kura::fs::read(&host).ok() } else { Some(Vec::new()) };
        let Some(bytes) = bytes else {
            mem::heap_free(key);
            return Err(McErr::FileNotFound);
        };
        match sound(bytes) {
            Some(x) => x,
            None => {
                mem::heap_free(key);
                if crate::files::resolve(&full).is_some() && kind != Kind::Wave {
                    eprintln!("sake: mci {element}: not played (no .vnv to decode, see masu pack)");
                }
                return Err(McErr::InvalidFile);
            }
        }
    };
    if let Media::Sound { data, size, fmt } = media {
        crate::sound::music_start(key, data, size, fmt, 0, 0);
        crate::sound::music_set(key, Some(0.0), Some(false));
    }
    let id = with(|s| {
        let id = s.next;
        s.next += 1;
        s.devs.insert(
            id,
            Dev { kind, alias, element: element.to_string(), media, key, tl: Timeline::new(duration), duration, format: TimeFormat::Ms, notes: Notes::default(), repeat: false, gen: 0, hwnd: 0, dst: None, volume: 0, mciwnd: 0 },
        );
        id
    });
    Ok(id)
}

/// A sound file's samples in guest memory (PCM as it is; the rest decoded by the page), and its
/// length. A sound the page can't decode (or no page) still opens, silent, with its length.
/// A sound file's PCM for the mixer, as MCI's waveaudio / mpegvideo play it: (the PCM in guest
/// memory, its size and format, or None when nothing here decodes it), its sample rate, its
/// duration. DirectShow's graph plays sound-only files with it (dshow.rs).
pub(crate) fn sound_pcm(b: Vec<u8>) -> Option<(Option<(u32, u32, crate::sound::Fmt)>, u32, u64)> {
    let (m, d) = sound(b)?;
    Some(match m {
        Media::Sound { data, size, fmt } => (Some((data, size, fmt)), fmt.rate, d),
        Media::Silent { rate, .. } => (None, rate, d),
        Media::Movie { .. } => return None,
    })
}

/// A sound's PCM block goes: its decoding stops, then the block is freed.
pub(crate) fn free_pcm(data: u32) {
    crate::sound::fill_drop(data);
    mem::heap_free(data);
}

/// The sound file `b` as Media: PCM copied into guest memory, the rest decoded while it plays.
/// A compressed sound (MP3, AAC, WMA ...) gets its whole block (16-bit, the track's length) at
/// once and a thread that decodes it there a few seconds ahead of the mixer; the open does not
/// wait for it, and the file's bytes are dropped when it ends.
fn sound(b: Vec<u8>) -> Option<(Media, u64)> {
    use crate::audiofile::Codec;
    let t = crate::audiofile::open(&b)?;
    let frames = t.total_frames();
    let duration = frames * SEC / t.rate.max(1) as u64;
    match &t.codec {
        Codec::Pcm { bits, float } => {
            let first = t.packets.first()?;
            let last = t.packets.last()?;
            let (start, end) = (first.0 as usize, (last.0 + last.1 as u64) as usize);
            let pcm = b.get(start..end.min(b.len()))?;
            let p = mem::heap_alloc(pcm.len() as u32, false);
            if p == 0 && !pcm.is_empty() {
                return None;
            }
            mem::write(p, pcm);
            let fmt = crate::sound::Fmt { rate: t.rate, channels: t.channels, bits: *bits as u32, float: *float, law: 0 };
            Some((Media::Sound { data: p, size: pcm.len() as u32, fmt }, duration))
        }
        Codec::Page { codec, desc, .. } => {
            let block = t.channels * 2;
            let silent = (Media::Silent { rate: t.rate, block }, duration);
            let size = frames.saturating_mul(block as u64);
            if size == 0 || size >= mem::GUEST_TOP as u64 {
                return Some(silent);
            }
            let Some(dec) = kura::adec::open(codec, t.rate, t.channels, desc) else { return Some(silent) };
            // (zeros until decoded: the mixer waits for them anyway)
            let p = mem::heap_alloc(size as u32, true);
            if p == 0 {
                kura::adec::close(dec);
                return Some(silent);
            }
            let fill = crate::sound::fill_start(p);
            let job = Decode { dec, data: p, total: frames as u32, channels: t.channels, rate: t.rate, packets: t.packets, frames: t.frames, file: b };
            let spawned = std::thread::Builder::new().name("mci decode".into()).spawn(move || job.run(&fill));
            if spawned.is_err() {
                // (no thread: nothing decodes it, the block plays silent)
                crate::sound::fill_drop(p);
            }
            let fmt = crate::sound::Fmt { rate: t.rate, channels: t.channels, bits: 16, float: false, law: 0 };
            Some((Media::Sound { data: p, size: size as u32, fmt }, duration))
        }
        // (`open` gives neither: they are Media Foundation's, `open_more`)
        Codec::Vorbis { .. } | Codec::Wma { .. } => None,
    }
}

/// A compressed sound being decoded into its guest block (see `sound`).
struct Decode {
    dec: u32,
    data: u32,
    total: u32,
    channels: u32,
    rate: u32,
    packets: Vec<(u64, u32)>,
    frames: Vec<u32>,
    file: Vec<u8>,
}

/// How far the packets handed to the decoder are ahead of what is written (seconds)
const DECODE_AHEAD: u64 = 8;
/// A decoder that gives nothing for this long has stopped (the rest stays silent)
const DECODE_STALL: Duration = Duration::from_secs(20);

impl Decode {
    fn run(self, fill: &crate::sound::Fill) {
        kura::snap::register("mci decode", false, 0);
        let ahead = self.rate as u64 * DECODE_AHEAD;
        let ch = self.channels as usize;
        let (mut next, mut pushed) = (0usize, 0u64);
        let mut written = 0u32;
        let mut pcm: Vec<f32> = Vec::new();
        let mut last = std::time::Instant::now();
        loop {
            // more packets while the decoder is less than DECODE_AHEAD ahead
            let from = next;
            while next < self.packets.len() && pushed < written as u64 + ahead {
                pushed += self.frames[next] as u64;
                next += 1;
            }
            let b = &self.file;
            kura::adec::push(self.dec, self.packets[from..next].iter().filter_map(|&(o, n)| b.get(o as usize..(o as usize).checked_add(n as usize)?)));
            kura::snap::idle();
            let all = kura::adec::wait(self.dec, Duration::from_millis(50));
            kura::snap::busy();
            pcm.clear();
            kura::adec::take(self.dec, usize::MAX, &mut pcm);
            let n = (pcm.len() / ch.max(1)).min((self.total - written) as usize) as u32;
            if n > 0 {
                last = std::time::Instant::now();
                let at = self.data + written * self.channels * 2;
                let to = written + n;
                let live = fill.write(to, || {
                    let Some(out) = mem::guest_mut(at, n * self.channels * 2) else { return };
                    for (o, x) in out.chunks_exact_mut(2).zip(&pcm) {
                        o.copy_from_slice(&pcm_i16(*x).to_le_bytes());
                    }
                });
                if !live {
                    break;
                }
                written = to;
            }
            let ended = all && next == self.packets.len() && n == 0;
            if written >= self.total || ended || kura::adec::failed(self.dec) || last.elapsed() > DECODE_STALL {
                break;
            }
        }
        kura::adec::close(self.dec);
        fill.finish();
        kura::snap::unregister();
    }
}

/// A decoded sample as 16-bit PCM
fn pcm_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

fn close(id: u32) -> bool {
    let Some(d) = with(|s| s.devs.remove(&id)) else { return false };
    post_all(d.notes.clone().command(None, true), id);
    match d.media {
        Media::Movie { .. } => play::end(d.key),
        Media::Sound { data, .. } => {
            crate::sound::music_stop(d.key);
            free_pcm(data);
        }
        Media::Silent { .. } => {}
    }
    mem::heap_free(d.key);
    true
}

/// play [from] [to]: runs from `from` (else where it is; at the end: from the start) to `to`
/// (else the end).
fn play_dev(id: u32, from: Option<u32>, to: Option<u32>, repeat: bool) -> Result<(), McErr> {
    let Some((f, tt, dur)) = dev(id, |d| (from.map(|v| d.from_fmt(v)), to.map(|v| d.from_fmt(v)), d.duration)) else { return Err(McErr::InvalidDeviceId) };
    if let (Some(a), Some(b)) = (f, tt) {
        if a > b {
            return Err(McErr::OutOfRange);
        }
    }
    dev(id, |d| {
        d.repeat = repeat;
        d.gen &= 0x7fff_ffff;
    });
    timeline(id, |tl, t| {
        let at = tl.position(t);
        let from = f.unwrap_or(if at >= tl.stop || at >= dur { 0 } else { at });
        tl.set_stop(t, tt.unwrap_or(dur));
        tl.seek(t, from);
        tl.run(t);
    });
    sync_sound(id);
    sync_movie(id);
    Ok(())
}

fn stop_dev(id: u32, pause: bool) {
    timeline(id, |tl, t| if pause { tl.pause(t) } else { tl.stop(t) });
    dev(id, |d| {
        if pause {
            d.gen |= 0x8000_0000;
        } else {
            d.gen &= 0x7fff_ffff;
        }
    });
    disarm(id);
    sync_sound(id);
}

fn seek_dev(id: u32, to: u64) {
    timeline(id, |tl, t| {
        tl.stop(t);
        tl.seek(t, to);
    });
    dev(id, |d| d.gen &= 0x7fff_ffff);
    disarm(id);
    sync_sound(id);
}

fn position(id: u32) -> u64 {
    timeline(id, |tl, t| tl.position(t)).unwrap_or(0)
}

/// Waits (MCI_WAIT) until the play of `id` ends.
fn wait_end(id: u32) {
    loop {
        let left = timeline(id, |tl, t| tl.until_end(t)).flatten();
        match left {
            Some(l) if l > 0 => {
                kura::snap::sleep(Duration::from_nanos(l * 100).min(Duration::from_millis(100)));
                crate::snap::point();
            }
            _ => break,
        }
    }
    timeline(id, |tl, t| tl.complete(t));
    sync_sound(id);
}

// ---------------------------------------------------------------------------------------------
// mciSendCommand

/// A parameter block's field after `ptrs` pointer-sized fields and then `dwords` DWORDs.
fn field(p: u32, ptrs: u32, dwords: u32) -> u32 {
    p + ptrs * mem::ps() + dwords * 4
}
fn aligned(off: u32) -> u32 {
    let ps = mem::ps();
    off.div_ceil(ps) * ps
}

fn send_command(c: &mut Ctx, id: u32, msg: u32, flags: u32, p: u32, wide: bool) -> Result<(), McErr> {
    let cb = if p != 0 { mem::rp(p) } else { 0 };
    let notify = (flags & MCI_NOTIFY != 0).then_some(cb);
    if msg == MCI_OPEN {
        if p == 0 {
            return Err(McErr::NullParameterBlock);
        }
        let ps = mem::ps();
        // MCI_OPEN_PARMS { dwCallback, wDeviceID, lpstrDeviceType, lpstrElementName, lpstrAlias }
        let id_at = p + ps;
        let type_at = p + aligned(ps + 4);
        let elem_at = type_at + ps;
        let alias_at = elem_at + ps;
        let s = |a: u32| {
            let v = mem::rp(a);
            if v == 0 { String::new() } else { crate::k32::arg_str(v, wide) }
        };
        let kind = if flags & MCI_OPEN_TYPE_ID != 0 {
            match kind_of_id(mem::rp(type_at)) {
                Ok(k) => Some(k),
                Err(e) => return Err(e),
            }
        } else if flags & MCI_OPEN_TYPE != 0 {
            match kind_of_type(&s(type_at)) {
                Some(Ok(k)) => Some(k),
                Some(Err(e)) => return Err(e),
                None => return Err(McErr::InvalidDeviceName),
            }
        } else {
            None
        };
        let element = if flags & MCI_OPEN_ELEMENT != 0 { s(elem_at) } else { String::new() };
        let alias = (flags & MCI_OPEN_ALIAS != 0).then(|| s(alias_at));
        let id = match open(kind, &element, alias.as_deref()) {
            Ok(id) => id,
            Err(e) => return Err(e),
        };
        w32(id_at, id);
        // MCI_DGV_OPEN_PARMS: dwStyle, hWndParent after the alias
        if flags & MCI_DGV_OPEN_PARENT != 0 {
            let style_at = alias_at + ps;
            let parent = mem::rp(style_at + ps);
            let child = flags & MCI_DGV_OPEN_WS != 0 && r32(style_at) & 0x4000_0000 != 0;
            dev(id, |d| d.hwnd = if child || parent != 0 { parent } else { 0 });
        }
        post_all(notify.map(|w| vec![(w, MCI_NOTIFY_SUCCESSFUL)]).unwrap_or_default(), id);
        return Ok(());
    }
    if id == MCI_ALL_DEVICE_ID && msg == MCI_CLOSE {
        let ids: Vec<u32> = with(|s| s.devs.keys().copied().collect());
        for i in ids {
            close(i);
        }
        return Ok(());
    }
    if dev(id, |_| ()).is_none() {
        return Err(McErr::InvalidDeviceId);
    }
    let interrupts = matches!(msg, MCI_STOP | MCI_SEEK | MCI_CLOSE | MCI_PLAY);
    let out = dev(id, |d| d.notes.command(notify, interrupts)).unwrap_or_default();
    post_all(out, id);
    let r = command(c, id, msg, flags, p, wide);
    if r.is_ok() {
        if let Some(w) = notify {
            if msg == MCI_PLAY {
                dev(id, |d| d.notes.wait_end(w));
            } else {
                post_all(vec![(w, MCI_NOTIFY_SUCCESSFUL)], id);
            }
        }
        if msg == MCI_PLAY {
            if flags & MCI_WAIT != 0 {
                wait_end(id);
                let out = dev(id, |d| d.notes.end()).unwrap_or_default();
                post_all(out, id);
            } else {
                arm(id);
            }
        }
    } else if let Some(w) = notify {
        post_all(vec![(w, MCI_NOTIFY_FAILURE)], id);
    }
    r
}

fn command(_c: &mut Ctx, id: u32, msg: u32, flags: u32, p: u32, wide: bool) -> Result<(), McErr> {
    let need = |p: u32| p != 0;
    match msg {
        MCI_CLOSE => {
            close(id);
            Ok(())
        }
        MCI_PLAY => {
            // MCI_PLAY_PARMS { dwCallback, dwFrom, dwTo }
            let from = (flags & MCI_FROM != 0 && need(p)).then(|| r32(field(p, 1, 0)));
            let to = (flags & MCI_TO != 0 && need(p)).then(|| r32(field(p, 1, 1)));
            play_dev(id, from, to, flags & MCI_DGV_PLAY_REPEAT != 0)
        }
        MCI_STOP => {
            stop_dev(id, false);
            Ok(())
        }
        MCI_PAUSE => {
            if mode(id) == Mode::Play {
                stop_dev(id, true);
            }
            Ok(())
        }
        MCI_RESUME => {
            if dev(id, |d| d.paused()).unwrap_or(false) {
                dev(id, |d| d.gen &= 0x7fff_ffff);
                timeline(id, |tl, t| tl.run(t));
                sync_sound(id);
                arm(id);
            }
            Ok(())
        }
        MCI_SEEK => {
            let to = if flags & MCI_SEEK_TO_START != 0 {
                0
            } else if flags & MCI_SEEK_TO_END != 0 {
                dev(id, |d| d.duration).unwrap_or(0)
            } else if flags & MCI_TO != 0 && need(p) {
                let v = r32(field(p, 1, 0));
                let (t, dur) = dev(id, |d| (d.from_fmt(v), d.duration)).unwrap_or((0, 0));
                if dev(id, |d| d.to_fmt(dur)).unwrap_or(0) < v {
                    return Err(McErr::OutOfRange);
                }
                t
            } else {
                return Err(McErr::MissingParameter);
            };
            seek_dev(id, to);
            Ok(())
        }
        MCI_STATUS => {
            if !need(p) {
                return Err(McErr::NullParameterBlock);
            }
            if flags & MCI_STATUS_ITEM == 0 {
                return Err(McErr::MissingParameter);
            }
            // MCI_STATUS_PARMS { dwCallback, dwReturn, dwItem, dwTrack }
            let ret = field(p, 1, 0);
            let Some(item) = Item::from_code(r32(field(p, 2, 0))) else { return Err(McErr::UnsupportedFunction) };
            mem::wp(ret, status(id, item));
            Ok(())
        }
        // (these read or write their parameter block: none is McErr::NullParameterBlock, as
        // Windows, not low memory)
        MCI_SET | MCI_SETAUDIO | MCI_WINDOW | MCI_PUT | MCI_WHERE | MCI_INFO if !need(p) => Err(McErr::NullParameterBlock),
        MCI_SET => {
            if flags & MCI_SET_TIME_FORMAT != 0 {
                let Some(f) = TimeFormat::from_code(r32(field(p, 1, 0))) else { return Err(McErr::BadTimeFormat) };
                let ok = dev(id, |d| format_ok(d.kind, d.video(), f)).unwrap_or(false);
                if !ok {
                    return Err(McErr::BadTimeFormat);
                }
                dev(id, |d| d.format = f);
            }
            Ok(())
        }
        MCI_SETAUDIO => {
            // MCI_DGV_SETAUDIO_PARMS { dwCallback, dwItem, dwValue, ... }
            if flags & MCI_DGV_SETAUDIO_ITEM != 0 && flags & MCI_DGV_SETAUDIO_VALUE != 0 && r32(field(p, 1, 0)) == MCI_DGV_SETAUDIO_VOLUME {
                set_volume(id, volume_db(r32(field(p, 1, 1))));
            }
            Ok(())
        }
        MCI_WINDOW => {
            // MCI_DGV_WINDOW_PARMS { dwCallback, hWnd, ... }
            if flags & MCI_DGV_WINDOW_HWND != 0 {
                let h = mem::rp(field(p, 1, 0));
                dev(id, |d| d.hwnd = h);
                sync_movie(id);
            }
            Ok(())
        }
        MCI_PUT => {
            // MCI_DGV_RECT_PARMS { dwCallback, RECT rc } (right, bottom: width, height)
            if flags & MCI_DGV_PUT_DESTINATION != 0 && flags & MCI_DGV_RECT != 0 {
                let r = field(p, 1, 0);
                let rect = (r32(r) as i32, r32(r + 4) as i32, r32(r + 8) as i32, r32(r + 12) as i32);
                dev(id, |d| d.dst = Some(rect));
                sync_movie(id);
            }
            Ok(())
        }
        MCI_WHERE => {
            if flags & MCI_DGV_WHERE_DESTINATION != 0 {
                let (dst, size) = dev(id, |d| (d.dst, match d.media { Media::Movie { size, .. } => size, _ => (0, 0) })).unwrap_or((None, (0, 0)));
                let (x, y, w, h) = dst.unwrap_or((0, 0, size.0 as i32, size.1 as i32));
                let r = field(p, 1, 0);
                for (i, v) in [x, y, w, h].into_iter().enumerate() {
                    w32(r + 4 * i as u32, v as u32);
                }
            }
            Ok(())
        }
        MCI_GETDEVCAPS => {
            if flags & MCI_GETDEVCAPS_ITEM == 0 || !need(p) {
                return Err(McErr::MissingParameter);
            }
            let item = r32(field(p, 2, 0));
            let (kind, video) = dev(id, |d| (d.kind, d.video())).unwrap_or((Kind::Mpeg, false));
            let v = match item {
                // CAN_RECORD, HAS_AUDIO, HAS_VIDEO, DEVICE_TYPE, USES_FILES, COMPOUND_DEVICE, CAN_EJECT, CAN_PLAY, CAN_SAVE
                1 => 0,
                2 => 1,
                3 => video as u32,
                4 => if kind == Kind::Wave { DEVTYPE_WAVEFORM_AUDIO } else { DEVTYPE_DIGITAL_VIDEO },
                5 | 6 => 1,
                7 => 0,
                8 => 1,
                9 => 0,
                _ => return Err(McErr::UnsupportedFunction),
            };
            mem::wp(field(p, 1, 0), v);
            Ok(())
        }
        MCI_INFO => {
            // MCI_INFO_PARMS { dwCallback, lpstrReturn, dwRetSize }; MCI_INFO_FILE 0x200, PRODUCT 0x100
            let (buf, size) = (mem::rp(field(p, 1, 0)), r32(field(p, 2, 0)));
            let (kind, el) = dev(id, |d| (d.kind, d.element.clone())).unwrap_or((Kind::Mpeg, String::new()));
            let s = if flags & 0x200 != 0 { el } else { product(kind).to_string() };
            crate::k32::ret_str(buf, size, &s, wide);
            Ok(())
        }
        MCI_CUE => Ok(()),
        _ => Err(McErr::UnsupportedFunction),
    }
}

fn product(kind: Kind) -> &'static str {
    match kind {
        Kind::Wave => "WAVE Audio and Video",
        Kind::Avi => "Video for Windows",
        Kind::Mpeg => "DirectShow",
    }
}

/// MCI_STATUS_* items a device here answers
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Item {
    Length,
    Position,
    Tracks,
    Mode,
    MediaPresent,
    TimeFormat,
    Ready,
    CurrentTrack,
    Volume,
}
impl Item {
    const ALL: [Item; 9] = [Item::Length, Item::Position, Item::Tracks, Item::Mode, Item::MediaPresent, Item::TimeFormat, Item::Ready, Item::CurrentTrack, Item::Volume];
    fn code(self) -> u32 {
        match self {
            Item::Length => MCI_STATUS_LENGTH,
            Item::Position => MCI_STATUS_POSITION,
            Item::Tracks => MCI_STATUS_NUMBER_OF_TRACKS,
            Item::Mode => MCI_STATUS_MODE,
            Item::MediaPresent => MCI_STATUS_MEDIA_PRESENT,
            Item::TimeFormat => MCI_STATUS_TIME_FORMAT,
            Item::Ready => MCI_STATUS_READY,
            Item::CurrentTrack => MCI_STATUS_CURRENT_TRACK,
            Item::Volume => MCI_DGV_STATUS_VOLUME,
        }
    }
    fn from_code(v: u32) -> Option<Item> {
        Item::ALL.into_iter().find(|x| x.code() == v)
    }
}

fn status(id: u32, item: Item) -> u32 {
    match item {
        Item::Length => dev(id, |d| d.to_fmt(d.duration)).unwrap_or(0),
        Item::Position => {
            let p = position(id);
            dev(id, |d| d.to_fmt(p)).unwrap_or(0)
        }
        Item::Tracks | Item::CurrentTrack => 1,
        Item::Mode => mode(id).code(),
        Item::MediaPresent | Item::Ready => 1,
        Item::TimeFormat => dev(id, |d| d.format.code()).unwrap_or(0),
        Item::Volume => dev(id, |d| volume_mci(d.volume)).unwrap_or(0),
    }
}

fn set_volume(id: u32, db: i32) {
    let Some((key, video)) = dev(id, |d| {
        d.volume = db;
        (d.key, d.video())
    }) else {
        return;
    };
    if video {
        sync_movie(id);
    } else {
        crate::sound::music_volume(key, db);
    }
}

/// An MCI answer as the program gets it: 0 or the error's number
fn mmresult(r: Result<(), McErr>) -> u32 {
    r.err().map_or(0, McErr::code)
}
fn mciSendCommandA(c: &mut Ctx) -> u32 {
    let (id, msg, flags, p) = (c.a(0), c.a(1), c.a(2), c.a(3));
    mmresult(send_command(c, id, msg, flags, p, false))
}
fn mciSendCommandW(c: &mut Ctx) -> u32 {
    let (id, msg, flags, p) = (c.a(0), c.a(1), c.a(2), c.a(3));
    mmresult(send_command(c, id, msg, flags, p, true))
}

// ---------------------------------------------------------------------------------------------
// mciSendString

fn device_id(name: &str) -> u32 {
    let n = name.to_ascii_lowercase();
    with(|s| s.devs.iter().find(|(_, d)| d.alias == n || d.element.to_ascii_lowercase() == n).map_or(0, |(&i, _)| i))
}

fn int(w: Option<&String>) -> Result<u32, McErr> {
    let w = w.ok_or(McErr::MissingParameter)?;
    w.parse::<u32>().map_err(|_| McErr::BadInteger)
}

/// Runs a command string; the text it returns.
fn send_string(c: &mut Ctx, cmd: &str, cb: u32) -> Result<String, McErr> {
    let Some(w) = words(cmd) else { return Err(McErr::MissingStringArgument) };
    let lw: Vec<String> = w.iter().map(|x| x.to_ascii_lowercase()).collect();
    let Some(verb) = lw.first() else { return Err(McErr::MissingStringArgument) };
    let Some(name) = w.get(1) else { return Err(McErr::MissingParameter) };
    let rest = &lw[2..];
    let has = |k: &str| rest.iter().any(|x| x == k);
    // (the word after keyword `k`, as an index into `w`)
    let after = |k: &str| rest.iter().position(|x| x == k).map(|i| i + 3);
    let mut flags = 0;
    if has("notify") {
        flags |= MCI_NOTIFY;
    }
    if has("wait") {
        flags |= MCI_WAIT;
    }
    let notify = (flags & MCI_NOTIFY != 0).then_some(cb);
    if verb == "open" {
        // open <element or device> [type t] [alias a] [shareable] [wait|notify] [style child] [parent h]
        let kind = match after("type").map(|i| w.get(i)) {
            Some(Some(t)) => match kind_of_type(t) {
                Some(Ok(k)) => Some(k),
                Some(Err(e)) => return Err(e),
                None => return Err(McErr::InvalidDeviceName),
            },
            Some(None) => return Err(McErr::MissingParameter),
            None => None,
        };
        // (a bare device name: "open waveaudio" has no element)
        let (kind, element) = match (kind, kind_of_type(name)) {
            (None, Some(Ok(k))) => (Some(k), String::new()),
            (None, Some(Err(e))) => return Err(e),
            (k, _) => (k, name.clone()),
        };
        let alias = after("alias").and_then(|i| w.get(i)).map(|s| s.as_str());
        let id = open(kind, &element, alias)?;
        if let Some(h) = after("parent").and_then(|i| w.get(i)).and_then(|s| s.parse::<u32>().ok()) {
            dev(id, |d| d.hwnd = h);
        }
        post_all(notify.map(|w| vec![(w, MCI_NOTIFY_SUCCESSFUL)]).unwrap_or_default(), id);
        return Ok(id.to_string());
    }
    if verb == "close" && lw[1] == "all" {
        let ids: Vec<u32> = with(|s| s.devs.keys().copied().collect());
        for i in ids {
            close(i);
        }
        return Ok(String::new());
    }
    // (the command first: an unknown one is that, whatever the device)
    const VERBS: [&str; 17] = ["play", "stop", "pause", "resume", "close", "cue", "seek", "status", "set", "setaudio", "window", "put", "where", "capability", "info", "open", "close"];
    if !VERBS.contains(&verb.as_str()) {
        return Err(McErr::UnrecognizedCommand);
    }
    let id = device_id(name);
    if id == 0 {
        return Err(McErr::InvalidDeviceName);
    }
    // the command as the parameter block mciSendCommand takes
    let ps = mem::ps();
    let p = mem::heap_alloc(64, true);
    mem::wp(p, cb);
    let mut ret = String::new();
    let r = (|| -> Result<(), McErr> {
        let msg = match verb.as_str() {
            "play" => {
                if let Some(i) = after("from") {
                    flags |= MCI_FROM;
                    w32(p + ps, int(w.get(i))?);
                }
                if let Some(i) = after("to") {
                    flags |= MCI_TO;
                    w32(p + ps + 4, int(w.get(i))?);
                }
                if has("repeat") {
                    flags |= MCI_DGV_PLAY_REPEAT;
                }
                MCI_PLAY
            }
            "stop" => MCI_STOP,
            "pause" => MCI_PAUSE,
            "resume" => MCI_RESUME,
            "close" => MCI_CLOSE,
            "cue" => MCI_CUE,
            "seek" => {
                match after("to").map(|i| lw.get(i).map(|s| s.as_str())) {
                    Some(Some("start")) => flags |= MCI_SEEK_TO_START,
                    Some(Some("end")) => flags |= MCI_SEEK_TO_END,
                    Some(_) => {
                        flags |= MCI_TO;
                        w32(p + ps, int(after("to").and_then(|i| w.get(i)))?);
                    }
                    None => return Err(McErr::MissingParameter),
                }
                MCI_SEEK
            }
            "status" => {
                let item = rest.iter().filter(|x| *x != "notify" && *x != "wait").cloned().collect::<Vec<_>>().join(" ");
                let it = match item.as_str() {
                    "length" => Item::Length,
                    "position" => Item::Position,
                    "mode" => Item::Mode,
                    "ready" => Item::Ready,
                    "media present" => Item::MediaPresent,
                    "time format" => Item::TimeFormat,
                    "number of tracks" => Item::Tracks,
                    "current track" => Item::CurrentTrack,
                    "volume" => Item::Volume,
                    "" => return Err(McErr::MissingParameter),
                    _ => return Err(McErr::UnrecognizedKeyword),
                };
                ret = match it {
                    Item::Mode => mode(id).name().into(),
                    Item::Ready | Item::MediaPresent => "true".into(),
                    Item::TimeFormat => format_name(dev(id, |d| d.format).unwrap_or(TimeFormat::Ms)).into(),
                    Item::Length | Item::Position | Item::Tracks | Item::CurrentTrack | Item::Volume => status(id, it).to_string(),
                };
                return Ok(());
            }
            "set" => {
                if let Some(i) = rest.iter().position(|x| x == "format").filter(|&i| i > 0 && rest[i - 1] == "time") {
                    let f = match rest.get(i + 1).map(|s| s.as_str()) {
                        Some("milliseconds") | Some("ms") => TimeFormat::Ms,
                        Some("frames") => TimeFormat::Frames,
                        Some("bytes") => TimeFormat::Bytes,
                        Some("samples") => TimeFormat::Samples,
                        Some(_) => return Err(McErr::BadTimeFormat),
                        None => return Err(McErr::MissingParameter),
                    };
                    flags |= MCI_SET_TIME_FORMAT;
                    w32(p + ps, f.code());
                }
                MCI_SET
            }
            "setaudio" => {
                if let Some(i) = after("to").filter(|_| has("volume")) {
                    flags |= MCI_DGV_SETAUDIO_ITEM | MCI_DGV_SETAUDIO_VALUE;
                    w32(p + ps, MCI_DGV_SETAUDIO_VOLUME);
                    w32(p + ps + 4, int(w.get(i))?);
                }
                MCI_SETAUDIO
            }
            "window" => {
                if let Some(i) = after("handle") {
                    let h = if lw.get(i).map(|s| s.as_str()) == Some("default") { 0 } else { int(w.get(i))? };
                    flags |= MCI_DGV_WINDOW_HWND;
                    mem::wp(p + ps, h);
                }
                MCI_WINDOW
            }
            "put" => {
                if let Some(i) = after("at").filter(|_| has("destination")) {
                    flags |= MCI_DGV_PUT_DESTINATION | MCI_DGV_RECT;
                    for k in 0..4 {
                        let v = w.get(i + k).ok_or(McErr::MissingParameter)?.parse::<i32>().map_err(|_| McErr::BadInteger)?;
                        w32(p + ps + 4 * k as u32, v as u32);
                    }
                }
                MCI_PUT
            }
            "where" => {
                flags |= MCI_DGV_WHERE_DESTINATION;
                let r = send_command(c, id, MCI_WHERE, flags, p, false);
                if r.is_ok() {
                    let v: Vec<String> = (0..4).map(|k| (r32(p + ps + 4 * k) as i32).to_string()).collect();
                    ret = v.join(" ");
                }
                return r;
            }
            "capability" => {
                let item = rest.join(" ");
                let (kind, video) = dev(id, |d| (d.kind, d.video())).unwrap_or((Kind::Mpeg, false));
                ret = match item.as_str() {
                    "can play" | "has audio" | "uses files" | "compound device" => "true".into(),
                    "can record" | "can eject" | "can save" => "false".into(),
                    "has video" | "can stretch" => if video { "true" } else { "false" }.into(),
                    "device type" => if kind == Kind::Wave { "waveaudio" } else { "digitalvideo" }.into(),
                    _ => return Err(McErr::UnrecognizedKeyword),
                };
                return Ok(());
            }
            "info" => {
                let (kind, el) = dev(id, |d| (d.kind, d.element.clone())).unwrap_or((Kind::Mpeg, String::new()));
                ret = if has("file") { el } else { product(kind).into() };
                return Ok(());
            }
            _ => return Err(McErr::UnrecognizedCommand),
        };
        send_command(c, id, msg, flags, p, false)
    })();
    mem::heap_free(p);
    match r {
        Ok(()) => Ok(ret),
        Err(e) => Err(e),
    }
}

fn format_name(f: TimeFormat) -> &'static str {
    match f {
        TimeFormat::Frames => "frames",
        TimeFormat::Bytes => "bytes",
        TimeFormat::Samples => "samples",
        TimeFormat::Ms => "milliseconds",
    }
}

fn mci_send_string(c: &mut Ctx, wide: bool) -> u32 {
    let (cmd, ret, size, cb) = (c.a(0), c.a(1), c.a(2), c.a(3));
    if cmd == 0 {
        return McErr::MissingStringArgument.code();
    }
    let s = crate::k32::arg_str(cmd, wide);
    match send_string(c, &s, cb) {
        Ok(text) => {
            if ret != 0 && size != 0 {
                crate::k32::ret_str(ret, size, &text, wide);
            }
            0
        }
        Err(e) => e.code(),
    }
}
fn mciSendStringA(c: &mut Ctx) -> u32 {
    mci_send_string(c, false)
}
fn mciSendStringW(c: &mut Ctx) -> u32 {
    mci_send_string(c, true)
}

fn mci_get_error_string(c: &mut Ctx, wide: bool) -> u32 {
    let (e, buf, size) = (c.a(0), c.a(1), c.a(2));
    if buf == 0 || size == 0 {
        return 0;
    }
    match error_text(e) {
        Some(t) => {
            crate::k32::ret_str(buf, size, t, wide);
            1
        }
        None => {
            crate::k32::ret_str(buf, size, "", wide);
            0
        }
    }
}
fn mciGetErrorStringA(c: &mut Ctx) -> u32 {
    mci_get_error_string(c, false)
}
fn mciGetErrorStringW(c: &mut Ctx) -> u32 {
    mci_get_error_string(c, true)
}
fn mciGetDeviceIDA(c: &mut Ctx) -> u32 {
    device_id(&crate::k32::arg_str(c.a(0), false))
}
fn mciGetDeviceIDW(c: &mut Ctx) -> u32 {
    device_id(&crate::k32::arg_str(c.a(0), true))
}

// ---------------------------------------------------------------------------------------------
// MCIWnd (msvfw32)

const WM_USER: u32 = 0x400;
const MCIWNDM_GETDEVICEID: u32 = WM_USER + 100;
const MCIWNDM_GETPOSITIONA: u32 = WM_USER + 102;
const MCIWNDM_GETSTART: u32 = WM_USER + 103;
const MCIWNDM_GETLENGTH: u32 = WM_USER + 104;
const MCIWNDM_GETEND: u32 = WM_USER + 105;
const MCIWNDM_GETMODEA: u32 = WM_USER + 106;
const MCIWNDM_SETVOLUME: u32 = WM_USER + 110;
const MCIWNDM_GETVOLUME: u32 = WM_USER + 111;
const MCIWNDM_SETREPEAT: u32 = WM_USER + 114;
const MCIWNDM_GETREPEAT: u32 = WM_USER + 115;
const MCIWNDM_PLAYFROM: u32 = WM_USER + 122;
const MCIWNDM_PLAYTO: u32 = WM_USER + 123;
const MCIWNDM_CAN_PLAY: u32 = WM_USER + 144;
const MCIWNDM_CAN_WINDOW: u32 = WM_USER + 145;
const MCIWNDM_OPENA: u32 = WM_USER + 153;
const MCIWNDM_NOTIFYMODE: u32 = WM_USER + 200;
const MCIWNDM_GETPOSITIONW: u32 = WM_USER + 202;
const MCIWNDM_GETMODEW: u32 = WM_USER + 206;
const MCIWNDM_OPENW: u32 = WM_USER + 252;
/// (our own: the waiter tells the window the play ended, on the window's thread)
const WM_MCIWND_MODE: u32 = WM_USER + 0x3f0;

const MCIWNDF_NOAUTOSIZEWINDOW: u32 = 1;
const MCIWNDF_NOTIFYMODE: u32 = 0x100;

#[derive(Default)]
struct Wnd {
    dev: u32,
    style: u32,
    parent: u32,
    mode: Mode,
    repeat: bool,
}

static WNDS: Mutex<Option<HashMap<u32, Wnd>>> = Mutex::new(None);
fn wnd<R>(h: u32, f: impl FnOnce(&mut Wnd) -> R) -> Option<R> {
    WNDS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).get_mut(&h).map(f)
}

fn gcall(c: &mut Ctx, dll: &str, name: &str, args: &[u32]) -> u32 {
    let f = crate::api::export(dll, name);
    c.call(f, args)
}

/// The mode an MCIWnd shows: of its device, Mode::NotReady with none.
fn wnd_mode(h: u32) -> Mode {
    match wnd(h, |w| w.dev).unwrap_or(0) {
        0 => Mode::NotReady,
        d => mode(d),
    }
}

/// MCIWNDM_NOTIFYMODE to the parent when the mode changed (MCIWNDF_NOTIFYMODE).
fn wnd_notify(c: &mut Ctx, h: u32) {
    let m = wnd_mode(h);
    let Some((parent, style, was)) = wnd(h, |w| (w.parent, w.style, std::mem::replace(&mut w.mode, m))) else { return };
    if was != m && style & MCIWNDF_NOTIFYMODE != 0 && parent != 0 {
        crate::user::send(c, parent, MCIWNDM_NOTIFYMODE, h, m.code());
    }
}

fn wnd_open(c: &mut Ctx, h: u32, file: &str) -> u32 {
    if let Some(d) = wnd(h, |w| std::mem::take(&mut w.dev)) {
        if d != 0 {
            close(d);
        }
    }
    let id = match open(None, file, Some(&format!("mciwnd{h:x}"))).or_else(|e| if e == McErr::ExtensionNotFound { open(Some(Kind::Mpeg), file, Some(&format!("mciwnd{h:x}"))) } else { Err(e) }) {
        Ok(id) => id,
        Err(e) => {
            wnd_notify(c, h);
            return e.code();
        }
    };
    let style = wnd(h, |w| {
        w.dev = id;
        w.style
    })
    .unwrap_or(0);
    dev(id, |d| {
        d.hwnd = h;
        d.mciwnd = h;
    });
    // the window takes the movie's size (its client area: the frame stays)
    if style & MCIWNDF_NOAUTOSIZEWINDOW == 0 {
        if let Some(size) = dev(id, |d| match d.media { Media::Movie { size, .. } => Some(size), _ => None }).flatten() {
            if let Some(r) = crate::user::client_rect_screen(h) {
                let wr = mem::heap_alloc(16, true);
                gcall(c, "user32", "GetWindowRect", &[h, wr]);
                let (fw, fh) = ((r32(wr + 8) as i32 - r32(wr) as i32) - (r.r - r.l), (r32(wr + 12) as i32 - r32(wr + 4) as i32) - (r.b - r.t));
                mem::heap_free(wr);
                // SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE
                gcall(c, "user32", "SetWindowPos", &[h, 0, 0, 0, (size.0 as i32 + fw) as u32, (size.1 as i32 + fh) as u32, 0x2 | 0x4 | 0x10]);
            }
        }
    }
    sync_movie(id);
    wnd_notify(c, h);
    0
}

fn wnd_command(c: &mut Ctx, h: u32, msg: u32, from: Option<u32>, to: Option<u32>) -> u32 {
    let Some(d) = wnd(h, |w| w.dev).filter(|&d| d != 0) else { return McErr::InvalidDeviceId.code() };
    let r = match msg {
        MCI_PLAY => {
            let repeat = wnd(h, |w| w.repeat).unwrap_or(false);
            let out = dev(d, |x| x.notes.command(None, true)).unwrap_or_default();
            post_all(out, d);
            match play_dev(d, from, to, repeat) {
                Ok(()) => {
                    arm(d);
                    0
                }
                Err(e) => e.code(),
            }
        }
        MCI_STOP => {
            stop_dev(d, false);
            0
        }
        MCI_PAUSE => {
            if mode(d) == Mode::Play {
                stop_dev(d, true);
            }
            0
        }
        MCI_RESUME => {
            if dev(d, |x| x.paused()).unwrap_or(false) {
                dev(d, |x| x.gen &= 0x7fff_ffff);
                timeline(d, |tl, t| tl.run(t));
                sync_sound(d);
                arm(d);
            }
            0
        }
        MCI_SEEK => {
            let t = dev(d, |x| x.from_fmt(from.unwrap_or(0))).unwrap_or(0);
            seek_dev(d, t);
            0
        }
        MCI_CLOSE => {
            wnd(h, |w| w.dev = 0);
            close(d);
            0
        }
        _ => McErr::UnsupportedFunction.code(),
    };
    wnd_notify(c, h);
    r
}

fn mciwnd_proc(c: &mut Ctx) -> u32 {
    let (h, msg, wp, lp) = (c.a(0), c.a(1), c.a(2), c.a(3));
    let devid = wnd(h, |w| w.dev).unwrap_or(0);
    match msg {
        // WM_CREATE: CREATESTRUCT's lpCreateParams is the file (or NULL)
        0x1 => {
            let cs = lp;
            // CREATESTRUCT { lpCreateParams, hInstance, hMenu, hwndParent, cy, cx, y, x, style, ... }
            let style = r32(cs + 4 * mem::ps() + 4 * 4);
            let parent = mem::rp(cs + 3 * mem::ps());
            let file = mem::rp(cs);
            WNDS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(h, Wnd { style: style & 0xffff, parent, mode: Mode::NotReady, ..Wnd::default() });
            if file != 0 {
                let f = crate::k32::arg_str(file, true);
                wnd_open(c, h, &f);
            }
            0
        }
        MCIWNDM_OPENA | MCIWNDM_OPENW => {
            if lp == 0 {
                return McErr::UnsupportedFunction.code();
            }
            let f = crate::k32::arg_str(lp, msg == MCIWNDM_OPENW);
            wnd_open(c, h, &f)
        }
        MCI_PLAY | MCI_STOP | MCI_PAUSE | MCI_RESUME | MCI_CLOSE => wnd_command(c, h, msg, None, None),
        MCI_SEEK => wnd_command(c, h, msg, Some(lp), None),
        MCIWNDM_PLAYFROM => wnd_command(c, h, MCI_PLAY, Some(lp), None),
        MCIWNDM_PLAYTO => wnd_command(c, h, MCI_PLAY, None, Some(lp)),
        WM_MCIWND_MODE => {
            wnd_notify(c, h);
            0
        }
        MCIWNDM_GETMODEA | MCIWNDM_GETMODEW => {
            let m = wnd_mode(h);
            if lp != 0 && wp != 0 {
                crate::k32::ret_str(lp, wp, m.name(), msg == MCIWNDM_GETMODEW);
            }
            m.code()
        }
        MCIWNDM_GETLENGTH | MCIWNDM_GETEND => dev(devid, |d| d.to_fmt(d.duration)).unwrap_or(0),
        MCIWNDM_GETSTART => 0,
        MCIWNDM_GETPOSITIONA | MCIWNDM_GETPOSITIONW => {
            let p = position(devid);
            dev(devid, |d| d.to_fmt(p)).unwrap_or(0)
        }
        MCIWNDM_GETDEVICEID => devid,
        MCIWNDM_CAN_PLAY => (devid != 0) as u32,
        MCIWNDM_CAN_WINDOW => dev(devid, |d| d.video()).unwrap_or(false) as u32,
        MCIWNDM_SETVOLUME => {
            set_volume(devid, volume_db(lp));
            0
        }
        MCIWNDM_GETVOLUME => dev(devid, |d| volume_mci(d.volume)).unwrap_or(0),
        MCIWNDM_SETREPEAT => {
            wnd(h, |w| w.repeat = lp != 0);
            dev(devid, |d| d.repeat = lp != 0);
            0
        }
        MCIWNDM_GETREPEAT => wnd(h, |w| w.repeat as u32).unwrap_or(0),
        // WM_SIZE / WM_MOVE: the movie follows the client area
        0x5 | 0x3 => {
            if devid != 0 {
                sync_movie(devid);
            }
            gcall(c, "user32", "DefWindowProcW", &[h, msg, wp, lp])
        }
        // WM_ERASEBKGND: black under the movie
        0x14 => {
            let rc = mem::heap_alloc(16, true);
            gcall(c, "user32", "GetClientRect", &[h, rc]);
            let br = gcall(c, "gdi32", "GetStockObject", &[4]);
            gcall(c, "user32", "FillRect", &[wp, rc, br]);
            mem::heap_free(rc);
            1
        }
        // WM_DESTROY: the device closes
        0x2 => {
            if devid != 0 {
                close(devid);
            }
            WNDS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).remove(&h);
            0
        }
        _ => gcall(c, "user32", "DefWindowProcW", &[h, msg, wp, lp]),
    }
}

/// "MCIWndClass", registered once (MCIWndRegisterClass, MCIWndCreate).
fn register_class(c: &mut Ctx) -> u32 {
    static DONE: OnceLock<()> = OnceLock::new();
    if DONE.get().is_none() {
        let proc_ = {
            static P: OnceLock<u32> = OnceLock::new();
            *P.get_or_init(|| crate::api::host_fn("mciwnd_proc", mciwnd_proc, 16))
        };
        let ps = mem::ps();
        // WNDCLASSW { style, lpfnWndProc, cbClsExtra, cbWndExtra, hInstance, hIcon, hCursor, hbrBackground, lpszMenuName, lpszClassName }
        let wc = mem::heap_alloc(80, true);
        let name: Vec<u16> = "MCIWndClass\0".encode_utf16().collect();
        let np = mem::heap_alloc(name.len() as u32 * 2, false);
        for (i, ch) in name.iter().enumerate() {
            mem::w16(np + 2 * i as u32, *ch);
        }
        // CS_DBLCLKS | CS_GLOBALCLASS
        w32(wc, 0x8 | 0x4000);
        let mut at = aligned(4);
        mem::wp(wc + at, proc_);
        at += ps;
        at += 8;
        at = aligned(at);
        for _ in 0..5 {
            at += ps;
        }
        mem::wp(wc + at, np);
        gcall(c, "user32", "RegisterClassW", &[wc]);
        mem::heap_free(wc);
        let _ = DONE.set(());
    }
    1
}

fn mciwnd_create(c: &mut Ctx, wide: bool) -> u32 {
    let (parent, inst, style, file) = (c.a(0), c.a(1), c.a(2), c.a(3));
    register_class(c);
    // the window's style: WS_CHILD with a parent, else an overlapped window; the MCIWNDF_ bits
    // ride in the low word, which the class reads back in WM_CREATE
    let ws = if style & 0xffff_0000 != 0 { style & 0xffff_0000 } else if parent != 0 { 0x5080_0000 } else { 0x10cf_0000 };
    let name: Vec<u16> = "MCIWndClass\0".encode_utf16().collect();
    let np = mem::heap_alloc(name.len() as u32 * 2, false);
    for (i, ch) in name.iter().enumerate() {
        mem::w16(np + 2 * i as u32, *ch);
    }
    // (the file as a UTF-16 string for the W window)
    let fp = if file != 0 {
        let f: Vec<u16> = crate::k32::arg_str(file, wide).encode_utf16().chain(std::iter::once(0)).collect();
        let p = mem::heap_alloc(f.len() as u32 * 2, false);
        for (i, ch) in f.iter().enumerate() {
            mem::w16(p + 2 * i as u32, *ch);
        }
        p
    } else {
        0
    };
    // CW_USEDEFAULT for a top-level one
    let (x, y, w, h) = if parent != 0 { (0, 0, 320, 240) } else { (0x8000_0000, 0x8000_0000, 320, 240) };
    let hwnd = gcall(c, "user32", "CreateWindowExW", &[0, np, 0, (ws & 0xffff_0000) | (style & 0xffff), x, y, w, h, parent, 0, inst, fp]);
    mem::heap_free(np);
    if fp != 0 {
        mem::heap_free(fp);
    }
    hwnd
}
fn MCIWndCreateA(c: &mut Ctx) -> u32 {
    mciwnd_create(c, false)
}
fn MCIWndCreateW(c: &mut Ctx) -> u32 {
    mciwnd_create(c, true)
}
fn MCIWndRegisterClass(c: &mut Ctx) -> u32 {
    register_class(c)
}

pub fn init() {
    for (n, f) in [
        ("mciSendCommandA", mciSendCommandA as Handler),
        ("mciSendCommandW", mciSendCommandW),
        ("mciSendStringA", mciSendStringA),
        ("mciSendStringW", mciSendStringW),
        ("mciGetErrorStringA", mciGetErrorStringA),
        ("mciGetErrorStringW", mciGetErrorStringW),
        ("mciGetDeviceIDA", mciGetDeviceIDA),
        ("mciGetDeviceIDW", mciGetDeviceIDW),
    ] {
        crate::api::implement("winmm", n, f);
    }
    for (n, f) in [("MCIWndCreateA", MCIWndCreateA as Handler), ("MCIWndCreate", MCIWndCreateA), ("MCIWndCreateW", MCIWndCreateW), ("MCIWndRegisterClass", MCIWndRegisterClass)] {
        crate::api::implement("msvfw32", n, f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Review b07: the commands that read or write a parameter block refuse a NULL one (the
    /// old code read, and MCI_WHERE wrote 16 bytes, at the bottom of the address space)
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn null_parameter_blocks_are_refused() {
        let _g = crate::test_guest::enter();
        let mut cpu = koji::Cpu::default();
        let mut c = Ctx { cpu: &mut cpu, esp: 0, edx: None, jumped: false, st0: None, trap: 0, this: None };
        for (msg, flags) in [
            (MCI_SET, MCI_SET_TIME_FORMAT),
            (MCI_SETAUDIO, MCI_DGV_SETAUDIO_ITEM | MCI_DGV_SETAUDIO_VALUE),
            (MCI_WINDOW, MCI_DGV_WINDOW_HWND),
            (MCI_PUT, MCI_DGV_PUT_DESTINATION | MCI_DGV_RECT),
            (MCI_WHERE, MCI_DGV_WHERE_DESTINATION),
            (MCI_INFO, 0x200),
            (MCI_STATUS, MCI_STATUS_ITEM),
        ] {
            assert_eq!(command(&mut c, 0x7777, msg, flags, 0, false), Err(McErr::NullParameterBlock), "{msg:#x}");
        }
    }

    proptest! {
        /// Each device's end check is a timer of its own for any device id (ids past 0xffff
        /// were cut when the waiter got id << 16 | gen), inside the MCI keys
        #[test]
        fn end_checks_are_keyed_by_any_device(a in any::<u32>(), b in any::<u32>()) {
            let (ka, kb) = (crate::pool::key::MCI | a as u64, crate::pool::key::MCI | b as u64);
            prop_assert_eq!(ka == kb, a == b);
            prop_assert_eq!(ka >> 32, crate::pool::key::MCI >> 32);
        }
    }

    #[test]
    fn devices_by_type_and_extension() {
        assert_eq!(kind_of_type("MPEGVideo"), Some(Ok(Kind::Mpeg)));
        assert_eq!(kind_of_type("waveaudio"), Some(Ok(Kind::Wave)));
        assert_eq!(kind_of_type("sequencer"), Some(Err(McErr::CannotLoadDriver)));
        assert_eq!(kind_of_type("nonsense"), None);
        assert_eq!(kind_of_file("C:\\a\\b.WAV"), Some(Ok(Kind::Wave)));
        assert_eq!(kind_of_file("x.mpg"), Some(Ok(Kind::Mpeg)));
        assert_eq!(kind_of_file("x.avi"), Some(Ok(Kind::Avi)));
        // (no extension: the program must name the type: LiLiM's Data\Sound\se\lilim)
        assert_eq!(kind_of_file("Data\\Sound\\se\\lilim"), None);
        assert_eq!(kind_of_file("dir.d\\lilim"), None);
        assert_eq!(kind_of_file("x.mid"), Some(Err(McErr::CannotLoadDriver)));
    }

    #[test]
    fn time_formats() {
        // 2.5 s: 2500 ms, 25 frames at 10 fps, 22050 samples at 8820 Hz, bytes of 4-byte frames
        let t = 25_000_000;
        assert_eq!(to_format(t, TimeFormat::Ms, 0, 0, (0, 1)), 2500);
        assert_eq!(to_format(t, TimeFormat::Frames, 0, 0, (10, 1)), 25);
        assert_eq!(to_format(t, TimeFormat::Samples, 8820, 4, (0, 1)), 22050);
        assert_eq!(to_format(t, TimeFormat::Bytes, 8820, 4, (0, 1)), 88200);
        assert!(format_ok(Kind::Wave, false, TimeFormat::Bytes));
        assert!(!format_ok(Kind::Mpeg, false, TimeFormat::Bytes));
        assert!(!format_ok(Kind::Mpeg, false, TimeFormat::Frames));
        assert!(format_ok(Kind::Mpeg, true, TimeFormat::Frames));
        assert_eq!(TimeFormat::from_code(1), None);
    }

    #[test]
    fn volumes() {
        assert_eq!(volume_db(1000), 0);
        assert_eq!(volume_db(0), -10000);
        assert_eq!(volume_db(500), -602);
        assert_eq!(volume_mci(0), 1000);
        assert_eq!(volume_mci(-10000), 0);
    }

    #[test]
    fn command_words() {
        assert_eq!(words("open \"C:\\My Music\\a b.mp3\" type mpegvideo alias bgm").unwrap(), ["open", "C:\\My Music\\a b.mp3", "type", "mpegvideo", "alias", "bgm"]);
        assert_eq!(words("  play   bgm  from 0 notify ").unwrap(), ["play", "bgm", "from", "0", "notify"]);
        assert_eq!(words("open \"\" alias x").unwrap(), ["open", "", "alias", "x"]);
        assert_eq!(words("open \"x"), None);
    }

    #[test]
    fn notifications_as_windows() {
        let mut n = Notes::default();
        // play notify, then the end: successful
        n.wait_end(7);
        assert_eq!(n.end(), [(7, MCI_NOTIFY_SUCCESSFUL)]);
        assert_eq!(n.end(), []);
        // play notify, then stop without notify: aborted
        n.wait_end(7);
        assert_eq!(n.command(None, true), [(7, MCI_NOTIFY_ABORTED)]);
        // play notify, then status with notify: superseded
        n.wait_end(7);
        assert_eq!(n.command(Some(8), false), [(7, MCI_NOTIFY_SUPERSEDED)]);
        // play notify, then status without: still pending
        n.wait_end(7);
        assert_eq!(n.command(None, false), []);
        assert_eq!(n.pending, Some(7));
    }

    #[test]
    fn error_texts() {
        assert!(error_text(McErr::InvalidDeviceName.code()).unwrap().contains("not open"));
        assert!(error_text(McErr::FileNotFound.code()).is_some());
        assert_eq!(error_text(1234), None);
    }

    proptest! {
        /// Every report owed goes out exactly once, whatever the commands: the pending one is
        /// either still pending or was reported (successful, superseded or aborted), never twice
        #[test]
        fn each_play_reported_at_most_once(ops in proptest::collection::vec((0u8..4, any::<bool>(), 1u32..5), 0..40)) {
            let mut n = Notes::default();
            let mut owed = 0i32;
            let mut sent = 0i32;
            for (op, flag, w) in ops {
                match op {
                    0 => {
                        // play (interrupts), with notify or not
                        sent += n.command(flag.then_some(w), true).len() as i32;
                        if flag { n.wait_end(w); owed += 1; }
                    }
                    1 => sent += n.command(flag.then_some(w), false).len() as i32,
                    2 => sent += n.command(None, true).len() as i32,
                    _ => sent += n.end().len() as i32,
                }
                prop_assert_eq!(owed, sent + n.pending.is_some() as i32);
            }
        }

        /// Formats round trip: a value in the format, to time and back, stays (for frames and
        /// milliseconds exactly; samples and bytes to the frame)
        #[test]
        fn formats_round_trip(v in 0u32..10_000_000, rate in prop::sample::select(vec![8000u32, 11025, 22050, 44100, 48000]), block in prop::sample::select(vec![1u32, 2, 4]), fps in 1u32..61) {
            prop_assert_eq!(to_format(from_format(v, TimeFormat::Ms, rate, block, (fps, 1)), TimeFormat::Ms, rate, block, (fps, 1)), v);
            prop_assert_eq!(to_format(from_format(v, TimeFormat::Frames, rate, block, (fps, 1)), TimeFormat::Frames, rate, block, (fps, 1)), v);
            let s = to_format(from_format(v, TimeFormat::Samples, rate, block, (fps, 1)), TimeFormat::Samples, rate, block, (fps, 1));
            prop_assert!(v - s <= 1, "{} {}", v, s);
            let b = to_format(from_format(v, TimeFormat::Bytes, rate, block, (fps, 1)), TimeFormat::Bytes, rate, block, (fps, 1));
            prop_assert!(v - b <= 2 * block, "{} {}", v, b);
        }

        /// The volume map is monotonic and stays in range
        #[test]
        fn volume_monotonic(a in 0u32..=1000, b in 0u32..=1000) {
            if a <= b { prop_assert!(volume_db(a) <= volume_db(b)); }
            prop_assert!((-10000..=0).contains(&volume_db(a)));
            prop_assert!(volume_mci(volume_db(a)).abs_diff(a) <= 1 || a < 10);
        }

        /// Any command string splits without panicking, and unquoted words keep their text
        #[test]
        fn words_never_panic(s in "[ a-z0-9\"\\\\.]{0,40}") {
            if let Some(w) = words(&s) {
                if !s.contains('"') {
                    prop_assert_eq!(w, s.split_whitespace().map(String::from).collect::<Vec<_>>());
                }
            }
        }
    }
}

/// A compressed sound decoded while it plays: the open returns at once with the whole 16-bit
/// block, the decoding stays a few seconds ahead, writes what a model of the page gives, stops
/// at a dropped block, and the mixer waits at what is decoded
#[cfg(test)]
mod decode_tests {
    use super::{pcm_i16, sound, Media, DECODE_AHEAD};
    use crate::mem;
    use proptest::prelude::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// MPEG-1 layer III 44.1 kHz 128 kbps stereo frames, the body's first byte k (mod 256)
    fn mp3(frames: usize) -> Vec<u8> {
        (0..frames)
            .flat_map(|k| {
                let mut f = vec![0xff, 0xfb, 0x90, 0];
                f.resize(417, 0);
                f[4] = k as u8;
                f
            })
            .collect()
    }
    /// what the test page decodes frame k's channel c to
    fn value(k: u8, c: usize) -> f32 {
        (k as f32 - 128.0) / 128.0 + c as f32 / 4096.0
    }
    /// A page that decodes each frame to 1152 frames of `value`, counting the frames it took
    fn page(stop: Arc<AtomicBool>, hold: Arc<AtomicBool>, taken: Arc<AtomicUsize>) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if !hold.load(Ordering::Relaxed) {
                    kura::adec::test_page_pass(&mut |b, ch| {
                        taken.fetch_add(1, Ordering::Relaxed);
                        (0..1152).flat_map(|_| (0..ch as usize).map(move |c| value(b[4], c))).collect()
                    });
                }
                std::thread::sleep(Duration::from_micros(200));
            }
        })
    }
    fn wait_for(what: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(30);
        while !what() {
            assert!(Instant::now() < end, "timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn block(m: &Media) -> (u32, u32) {
        match m {
            Media::Sound { data, size, fmt } => {
                assert_eq!((fmt.bits, fmt.float, fmt.channels), (16, false, 2));
                (*data, *size)
            }
            _ => panic!("not a sound"),
        }
    }

    #[test]
    fn decoded_while_playing_as_the_page_gives() {
        let _a = crate::ADEC_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::test_guest::enter();
        kura::adec::test_page_pass(&mut |_, _| Vec::new());
        let (stop, hold, taken) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(true)), Arc::new(AtomicUsize::new(0)));
        let pg = page(stop.clone(), hold.clone(), taken.clone());
        // 600 frames: 15.7 s, more than DECODE_AHEAD
        let n = 600usize;
        let (m, duration) = sound(mp3(n)).unwrap();
        let (data, size) = block(&m);
        assert_eq!(size as usize, n * 1152 * 4);
        assert_eq!(duration, n as u64 * 1152 * 10_000_000 / 44100);
        let fill = crate::sound::fill_of_for_tests(data).unwrap();
        assert_eq!(fill.frames(), 0);
        // the open did not wait: nothing is decoded yet, and only DECODE_AHEAD is handed out
        std::thread::sleep(Duration::from_millis(100));
        hold.store(false, Ordering::Relaxed);
        wait_for(|| fill.done());
        assert_eq!(fill.frames() as usize, n * 1152);
        // (pushed ahead of what was written: at most DECODE_AHEAD + one batch, never all at once)
        assert_eq!(taken.load(Ordering::Relaxed), n);
        let pcm = mem::slice(data, size);
        for k in [0usize, 1, 299, 599] {
            for c in 0..2 {
                let at = (k * 1152 + 17) * 4 + c * 2;
                assert_eq!(i16::from_le_bytes([pcm[at], pcm[at + 1]]), pcm_i16(value(k as u8, c)), "frame {k} ch {c}");
            }
        }
        super::free_pcm(data);
        stop.store(true, Ordering::Relaxed);
        pg.join().unwrap();
    }

    #[test]
    fn only_a_few_seconds_are_handed_to_the_page_ahead() {
        let _a = crate::ADEC_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::test_guest::enter();
        kura::adec::test_page_pass(&mut |_, _| Vec::new());
        // a page that takes the chunks but decodes none: the decoder stops pushing at the limit
        let (m, _) = sound(mp3(2000)).unwrap();
        let (data, _) = block(&m);
        std::thread::sleep(Duration::from_millis(200));
        let mut chunks = 0;
        kura::adec::test_page_pass(&mut |_, _| {
            chunks += 1;
            Vec::new()
        });
        let limit = (44100 * DECODE_AHEAD).div_ceil(1152) as usize + 1;
        assert!(chunks > 0 && chunks <= limit, "{chunks} chunks pushed (limit {limit})");
        super::free_pcm(data);
    }

    #[test]
    fn a_dropped_block_is_not_written() {
        let _a = crate::ADEC_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::test_guest::enter();
        kura::adec::test_page_pass(&mut |_, _| Vec::new());
        let (stop, hold, taken) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)));
        let pg = page(stop.clone(), hold.clone(), taken.clone());
        for _ in 0..20 {
            let (m, _) = sound(mp3(3000)).unwrap();
            let (data, size) = block(&m);
            let fill = crate::sound::fill_of_for_tests(data).unwrap();
            wait_for(|| fill.frames() > 0);
            crate::sound::fill_drop(data);
            // (what the guest puts there next stays)
            mem::fill(data, size, 0x5a);
            std::thread::sleep(Duration::from_millis(20));
            assert!(mem::slice(data, size).iter().all(|&b| b == 0x5a));
            mem::heap_free(data);
        }
        stop.store(true, Ordering::Relaxed);
        pg.join().unwrap();
    }

    proptest! {
        /// the 16-bit sample of a decoded float: clamped, rounded, monotonic, exact on the grid
        #[test]
        fn i16_of_floats(x in any::<f32>(), y in -1.5f32..1.5, k in -32767i32..=32767) {
            let v = pcm_i16(x);
            if x.is_nan() { prop_assert_eq!(v, 0); }
            if x >= 1.0 { prop_assert_eq!(v, 32767); }
            if x <= -1.0 { prop_assert_eq!(v, -32767); }
            if !x.is_nan() && x <= y { prop_assert!(v <= pcm_i16(y)); }
            prop_assert_eq!(pcm_i16(k as f32 / 32767.0) as i32, k);
        }
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
#[path = "mci_snapshot_tests.rs"]
mod snapshot_tests;
