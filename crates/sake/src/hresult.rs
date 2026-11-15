//! The HRESULTs sake's COM code returns, named once (winerror.h's values; each module used to
//! keep its own copy of the same constants). A module's own closed set of failures (its `Hr`
//! enum) maps onto these at the handler's edge.
pub const S_OK: u32 = 0;
pub const S_FALSE: u32 = 1;
pub const E_NOTIMPL: u32 = 0x8000_4001;
pub const E_NOINTERFACE: u32 = 0x8000_4002;
pub const E_POINTER: u32 = 0x8000_4003;
pub const E_FAIL: u32 = 0x8000_4005;
pub const E_PENDING: u32 = 0x8000_000a;
pub const E_UNEXPECTED: u32 = 0x8000_ffff;
pub const E_HANDLE: u32 = 0x8007_0006;
pub const E_OUTOFMEMORY: u32 = 0x8007_000e;
pub const E_INVALIDARG: u32 = 0x8007_0057;
pub const E_NOT_SUFFICIENT_BUFFER: u32 = 0x8007_007a;
pub const E_PROP_ID_UNSUPPORTED: u32 = 0x8007_0490;
pub const DISP_E_MEMBERNOTFOUND: u32 = 0x8002_0003;
pub const DISP_E_PARAMNOTFOUND: u32 = 0x8002_0004;
pub const DISP_E_TYPEMISMATCH: u32 = 0x8002_0005;
pub const DISP_E_UNKNOWNNAME: u32 = 0x8002_0006;
pub const DISP_E_EXCEPTION: u32 = 0x8002_0009;
pub const DISP_E_BADPARAMCOUNT: u32 = 0x8002_000e;
pub const TYPE_E_ELEMENTNOTFOUND: u32 = 0x8002_802b;
pub const TYPE_E_CANTLOADLIBRARY: u32 = 0x8002_9c4a;
pub const STG_E_MEDIUMFULL: u32 = 0x8003_0070;
pub const CLASS_E_NOAGGREGATION: u32 = 0x8004_0110;
pub const REGDB_E_CLASSNOTREG: u32 = 0x8004_0154;
pub const CO_E_NOTINITIALIZED: u32 = 0x8004_01f0;
