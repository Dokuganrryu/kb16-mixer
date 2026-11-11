//! Control ids, and the Win32 message numbers the `windows` crate keeps in other modules.

use windows::Win32::UI::WindowsAndMessaging::WM_USER;

pub const ID_TABS: i32 = 10;
pub const ID_PAGE: i32 = 20; // + page
pub const ID_COLUMN: i32 = 30; // + column: channel name over a key column
pub const ID_KEY: i32 = 100; // + row * 4 + col
pub const ID_KNOB: i32 = 120; // + knob
pub const ID_CH_NAME: i32 = 200; // + channel, and the same for the rest of the channel ids
pub const ID_CH_KIND: i32 = 210;
pub const ID_CH_ABOUT: i32 = 220;
pub const ID_CH_LIST: i32 = 230;
pub const ID_CH_ADD: i32 = 240;
pub const ID_CH_REMOVE: i32 = 250;
pub const ID_CH_LABEL: i32 = 260; // + channel * 2 (+ 1)
pub const ID_NAME: i32 = 300;
pub const ID_BRIGHTNESS: i32 = 301;
pub const ID_STEP: i32 = 302;
pub const ID_DUCK: i32 = 303;
pub const ID_SHIFT: i32 = 304;
pub const ID_SCREEN_OFF: i32 = 305;
pub const ID_LIGHTS_DIM: i32 = 306;
pub const ID_AUTOSTART: i32 = 307;
pub const ID_OPEN_FILE: i32 = 308;
pub const ID_OPEN_FOLDER: i32 = 309;
pub const ID_VERSION: i32 = 310;
pub const ID_STATUS: i32 = 400;
pub const ID_CLOSE: i32 = 401;
/// Labels nothing refers to get ids from here up.
pub const ID_LABELS: i32 = 1000;

pub const CB_ADDSTRING: u32 = 0x0143;
pub const CB_GETCURSEL: u32 = 0x0147;
pub const CB_RESETCONTENT: u32 = 0x014B;
pub const CB_SETCURSEL: u32 = 0x014E;
pub const CB_SETDROPPEDWIDTH: u32 = 0x0160;
pub const LB_ADDSTRING: u32 = 0x0180;
pub const LB_RESETCONTENT: u32 = 0x0184;
pub const LB_GETCURSEL: u32 = 0x0188;
pub const EM_LIMITTEXT: u32 = 0x00C5;
pub const TBM_GETPOS: u32 = WM_USER;
pub const TBM_SETPOS: u32 = WM_USER + 5;
pub const TBM_SETRANGE: u32 = WM_USER + 6;
pub const TB_ENDTRACK: u32 = 8;
pub const BM_GETCHECK: u32 = 0x00F0;
pub const BM_SETCHECK: u32 = 0x00F1;
pub const TCM_SETCURSEL: u32 = 0x130C;
pub const TCM_GETCURSEL: u32 = 0x130B;
pub const TCM_INSERTITEMW: u32 = 0x133E;
pub const SS_CENTER: u32 = 1;
