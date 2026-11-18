# Use-case coverage

346 game runs, 3897 entry points they call, 6174 the tests call.

| kind | used by games | tested | by games weighted | by calls weighted |
|---|---|---|---|---|
| IAMStreamSelect | 3 | 1 | 33.3% | 25.0% |
| IBaseFilter | 9 | 9 | 100.0% | 100.0% |
| IEnumPins | 2 | 2 | 100.0% | 100.0% |
| IMediaSample | 13 | 0 | 0.0% | 0.0% |
| IMemAllocator | 4 | 0 | 0.0% | 0.0% |
| IMemInputPin | 5 | 0 | 0.0% | 0.0% |
| IPin | 11 | 8 | 83.9% | 98.8% |
| api | 3323 | 1542 | 79.4% | 87.3% |
| d3d | 254 | 134 | 66.3% | 76.0% |
| d3d9 | 141 | 111 | 86.5% | 98.1% |
| glmissing | 1 | 0 | 0.0% | 0.0% |
| stub | 131 | 43 | 40.5% | 0.4% |

## Called by games, reached by no test (1958)

| entry point | games | calls |
|---|---|---|
| `api:kernel32!InitializeCriticalSectionAndSpinCount` | 223 | 8066 |
| `api:kernel32!SetHandleCount` | 217 | 498 |
| `api:sake!IDirectSoundBuffer8::15` | 200 | 172971 |
| `api:kernel32!EncodePointer` | 197 | 144316 |
| `api:user32!SetCursor` | 146 | 25781 |
| `api:kernel32!DecodePointer` | 140 | 148942 |
| `api:sake!d3d9:2:57` | 104 | 3435800 |
| `api:sake!d3d9:2:43` | 103 | 159262 |
| `api:sake!d3d9:2:65` | 102 | 2866697 |
| `api:sake!d3d9:2:69` | 102 | 2580724 |
| `api:sake!d3d9:2:17` | 102 | 81138 |
| `api:sake!d3d9:1:8` | 102 | 1673 |
| `d3d9:IDirect3D9::GetAdapterDisplayMode` | 102 | 1673 |
| `api:sake!d3d9:5:19` | 98 | 34700 |
| `api:sake!d3d9:5:20` | 98 | 34662 |
| `api:sake!IDirectSoundBuffer8::16` | 93 | 1443 |
| `api:sake!d3d9:2:67` | 88 | 2952665 |
| `api:sake!d3d9:2:89` | 88 | 648995 |
| `api:sake!d3d9:1:14` | 80 | 131 |
| `api:user32!RegisterWindowMessageA` | 74 | 232 |
| `api:sake!d3d9:2:47` | 69 | 1063285 |
| `api:user32!ShowCursor` | 69 | 517 |
| `api:shell32!DragAcceptFiles` | 69 | 79 |
| `api:kernel32!InitializeSRWLock` | 66 | 32285 |
| `api:sake!d3d11:70:5` | 65 | 5571 |
| `api:sake!d3d11:70:3` | 64 | 7887 |
| `api:sake!d3d11:70:7` | 64 | 3937 |
| `api:sake!d3d11:70:0` | 64 | 267 |
| `api:sake!d3d11:74:2` | 63 | 319894 |
| `api:user32!PtInRect` | 62 | 464252 |
| `api:kernel32!LocaleNameToLCID` | 62 | 4476 |
| `api:sake!d3d11:70:20` | 62 | 700 |
| `api:sake!d3d11:71:14` | 61 | 299568 |
| `api:sake!d3d11:71:44` | 61 | 74871 |
| `api:sake!d3d11:71:15` | 60 | 299546 |
| `api:kernel32!InitializeConditionVariable` | 60 | 968 |
| `api:sake!d3d11:71:48` | 59 | 218665 |
| `api:sake!d3d11:71:9` | 59 | 94080 |
| `api:sake!d3d11:71:11` | 59 | 71947 |
| `api:sake!d3d11:71:8` | 58 | 471419 |
| `api:user32!WINNLSEnableIME` | 57 | 182 |
| `api:sake!d3d11:64:7` | 56 | 141 |
| `api:sake!d3d11:70:29` | 54 | 3813 |
| `d3d:70.29` | 54 | 3813 |
| `api:sake!d3d9:2:18` | 51 | 37811 |
| `api:sake!d3d11:70:6` | 51 | 114 |
| `api:sake!d3d11:68:2` | 51 | 55 |
| `d3d:68.2` | 51 | 55 |
| `api:sake!d3d11:71:23` | 50 | 25782 |
| `d3d:71.23` | 50 | 25782 |
| `api:sake!d3d9:1:5` | 50 | 82 |
| `d3d9:IDirect3D9::GetAdapterIdentifier` | 50 | 82 |
| `api:sake!d3d9:5:17` | 48 | 203301 |
| `d3d9:Texture::GetLevelDesc` | 48 | 203253 |
| `api:kernel32!GetLocaleInfoEx` | 47 | 784 |
| `api:sake!d3d11:71:0` | 47 | 89 |
| `api:sake!d3d11:64:0` | 47 | 87 |
| `api:sake!d3d11:68:7` | 47 | 78 |
| `d3d:68.7` | 47 | 78 |
| `api:sake!d3d11:70:33` | 46 | 6239 |

## Called by games, not implemented (132)

| entry point | games | calls |
|---|---|---|
| `stub:imm32!ImmGetContext` | 89 | 628 |
| `stub:imm32!ImmAssociateContext` | 73 | 85 |
| `stub:kernel32!InitializeProcThreadAttributeList` | 38 | 76 |
| `stub:kernel32!DeleteProcThreadAttributeList` | 38 | 38 |
| `stub:imm32!ImmReleaseContext` | 34 | 110 |
| `stub:user32!RegisterTouchWindow` | 34 | 34 |
| `stub:imm32!ImmGetOpenStatus` | 25 | 49 |
| `stub:imm32!ImmGetConversionStatus` | 23 | 68 |
| `stub:dwmapi!DwmSetWindowAttribute` | 22 | 74 |
| `stub:imm32!ImmSetOpenStatus` | 17 | 43 |
| `stub:kernel32!AttachConsole` | 12 | 12 |
| `stub:kernel32!SetConsoleOutputCP` | 11 | 11 |
| `stub:imm32!ImmIsIME` | 10 | 32 |
| `stub:kernel32!SetConsoleCP` | 10 | 10 |
| `stub:dwmapi!DwmEnableComposition` | 9 | 9 |
| `stub:setupapi!SetupDiEnumDeviceInterfaces` | 8 | 6528 |
| `stub:ole32!CoGetApartmentType` | 8 | 19 |
| `stub:uxtheme!RefreshImmersiveColorPolicyState` | 8 | 18 |
| `stub:uxtheme!SetPreferredAppMode` | 8 | 18 |
| `stub:user32!SwapMouseButton` | 8 | 16 |
| `stub:avifil32!AVIFileInit` | 8 | 9 |
| `stub:kernel32!SetConsoleMode` | 8 | 8 |
| `stub:user32!GetThreadDesktop` | 8 | 8 |
| `stub:kernel32!GetConsoleScreenBufferInfo` | 7 | 43 |
| `stub:user32!IsTouchWindow` | 7 | 24 |
| `stub:imm32!ImmSetCompositionFontA` | 7 | 11 |
| `stub:kernel32!SetConsoleTextAttribute` | 6 | 72 |
| `stub:winmm!waveInGetNumDevs` | 6 | 32 |
| `stub:uxtheme!ShouldAppsUseDarkMode` | 6 | 18 |
| `stub:crypt32!CertOpenSystemStoreA` | 6 | 6 |
| `stub:imm32!ImmCreateContext` | 6 | 6 |
| `stub:kernel32!GetConsoleProcessList` | 6 | 6 |
| `stub:kernel32!PowerCreateRequest` | 6 | 6 |
| `stub:kernel32!PowerSetRequest` | 6 | 6 |
| `stub:shell32!ExtractIconExA` | 6 | 6 |
| `stub:shell32!SetCurrentProcessExplicitAppUserModelID` | 6 | 6 |
| `stub:uiautomationcore!UiaLookupId` | 6 | 6 |
| `stub:user32!RegisterPowerSettingNotification` | 6 | 6 |
| `stub:uxtheme!FlushMenuThemes` | 6 | 6 |
| `stub:user32!GetClipCursor` | 5 | 55833 |
| `stub:winhttp!WinHttpGetIEProxyConfigForCurrentUser` | 5 | 9 |
| `stub:imm32!ImmDisableTextFrameService` | 5 | 5 |
| `stub:user32!GetCursorInfo` | 4 | 207231 |
| `glmissing:glDrawBuffers` | 4 | 919 |
| `stub:imm32!ImmNotifyIME` | 4 | 4 |
| `stub:shell32!InitNetworkAddressControl` | 4 | 4 |
| `stub:user32!SetGestureConfig` | 4 | 4 |
| `stub:winmm!joyReleaseCapture` | 4 | 4 |
| `stub:shell32!ExtractIconA` | 3 | 5 |
| `stub:ucrtbase!iswxdigit` | 2 | 5105 |
| `stub:imm32!ImmAssociateContextEx` | 2 | 125 |
| `stub:ucrtbase!iswalnum` | 2 | 63 |
| `stub:ucrtbase!logbf` | 2 | 5 |
| `stub:user32!SetMenuInfo` | 2 | 4 |
| `stub:crypt32!CertOpenSystemStoreW` | 2 | 2 |
| `stub:imm32!ImmGetProperty` | 2 | 2 |
| `stub:kernel32!CreateMemoryResourceNotification` | 2 | 2 |
| `stub:kernel32!GetConsoleWindow` | 2 | 2 |
| `stub:kernel32!GetGeoInfoW` | 2 | 2 |
| `stub:kernel32!GetUserGeoID` | 2 | 2 |
