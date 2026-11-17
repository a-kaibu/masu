# Use-case coverage

346 game runs, 3929 entry points they call, 6117 the tests call.

| kind | used by games | tested | by games weighted | by calls weighted |
|---|---|---|---|---|
| IAMStreamSelect | 3 | 1 | 33.3% | 25.0% |
| IBaseFilter | 9 | 9 | 100.0% | 100.0% |
| IEnumPins | 2 | 2 | 100.0% | 100.0% |
| IMediaSample | 13 | 0 | 0.0% | 0.0% |
| IMemAllocator | 4 | 0 | 0.0% | 0.0% |
| IMemInputPin | 5 | 0 | 0.0% | 0.0% |
| IPin | 11 | 8 | 83.9% | 98.9% |
| api | 3349 | 1546 | 79.3% | 87.3% |
| d3d | 259 | 136 | 66.4% | 83.3% |
| d3d9 | 141 | 111 | 86.4% | 98.0% |
| glmissing | 1 | 0 | 0.0% | 0.0% |
| stub | 132 | 44 | 40.7% | 0.4% |

## Called by games, reached by no test (1983)

| entry point | games | calls |
|---|---|---|
| `api:kernel32!InitializeCriticalSectionAndSpinCount` | 224 | 8098 |
| `api:kernel32!SetHandleCount` | 217 | 501 |
| `api:sake!IDirectSoundBuffer8::15` | 206 | 181784 |
| `api:kernel32!EncodePointer` | 197 | 144536 |
| `api:user32!SetCursor` | 151 | 25195 |
| `api:kernel32!DecodePointer` | 140 | 149379 |
| `api:sake!d3d9:2:57` | 104 | 3289876 |
| `api:sake!d3d9:2:43` | 103 | 154920 |
| `api:sake!d3d9:2:65` | 102 | 2863432 |
| `api:sake!d3d9:2:69` | 102 | 2667154 |
| `api:sake!d3d9:2:17` | 102 | 82756 |
| `api:sake!d3d9:1:8` | 102 | 1642 |
| `d3d9:IDirect3D9::GetAdapterDisplayMode` | 102 | 1642 |
| `api:sake!IDirectSoundBuffer8::16` | 99 | 1628 |
| `api:sake!d3d9:5:19` | 98 | 40147 |
| `api:sake!d3d9:5:20` | 98 | 40112 |
| `api:sake!d3d9:2:67` | 88 | 2458304 |
| `api:sake!d3d9:2:89` | 88 | 609724 |
| `api:sake!d3d9:1:14` | 80 | 131 |
| `api:user32!RegisterWindowMessageA` | 74 | 232 |
| `api:user32!ShowCursor` | 70 | 558 |
| `api:shell32!DragAcceptFiles` | 69 | 79 |
| `api:sake!d3d9:2:47` | 68 | 1153309 |
| `api:kernel32!InitializeSRWLock` | 66 | 32289 |
| `api:sake!d3d11:70:5` | 65 | 5801 |
| `api:sake!d3d11:70:3` | 64 | 24435 |
| `api:sake!d3d11:70:7` | 64 | 4036 |
| `api:sake!d3d11:70:0` | 64 | 267 |
| `api:sake!d3d11:74:2` | 63 | 703196 |
| `api:sake!d3d11:71:44` | 62 | 57890 |
| `api:kernel32!LocaleNameToLCID` | 62 | 4476 |
| `api:sake!d3d11:70:20` | 62 | 704 |
| `api:user32!PtInRect` | 61 | 421600 |
| `api:sake!d3d11:71:14` | 61 | 395192 |
| `api:sake!d3d11:71:15` | 60 | 395170 |
| `api:sake!d3d11:71:48` | 60 | 339059 |
| `api:sake!d3d11:71:9` | 60 | 135329 |
| `api:sake!d3d11:71:11` | 60 | 112740 |
| `api:kernel32!InitializeConditionVariable` | 60 | 972 |
| `api:sake!d3d11:71:8` | 59 | 837230 |
| `api:user32!WINNLSEnableIME` | 57 | 186 |
| `api:sake!d3d11:64:7` | 56 | 141 |
| `api:sake!d3d11:70:29` | 54 | 3893 |
| `d3d:70.29` | 54 | 3893 |
| `api:sake!d3d9:2:18` | 51 | 36044 |
| `api:sake!d3d11:70:6` | 51 | 114 |
| `api:sake!d3d11:68:2` | 51 | 55 |
| `d3d:68.2` | 51 | 55 |
| `api:sake!d3d11:71:23` | 50 | 42369 |
| `d3d:71.23` | 50 | 42369 |
| `api:sake!d3d9:1:5` | 50 | 82 |
| `d3d9:IDirect3D9::GetAdapterIdentifier` | 50 | 82 |
| `api:sake!d3d9:5:17` | 48 | 180818 |
| `d3d9:Texture::GetLevelDesc` | 48 | 180788 |
| `api:kernel32!GetLocaleInfoEx` | 47 | 784 |
| `api:sake!d3d11:71:0` | 47 | 89 |
| `api:sake!d3d11:64:0` | 47 | 87 |
| `api:sake!d3d11:68:7` | 47 | 78 |
| `d3d:68.7` | 47 | 78 |
| `api:sake!d3d11:70:24` | 46 | 16883 |

## Called by games, not implemented (133)

| entry point | games | calls |
|---|---|---|
| `stub:imm32!ImmGetContext` | 89 | 637 |
| `stub:imm32!ImmAssociateContext` | 73 | 85 |
| `stub:kernel32!InitializeProcThreadAttributeList` | 38 | 76 |
| `stub:kernel32!DeleteProcThreadAttributeList` | 38 | 38 |
| `stub:imm32!ImmReleaseContext` | 34 | 110 |
| `stub:user32!RegisterTouchWindow` | 34 | 34 |
| `stub:imm32!ImmGetOpenStatus` | 25 | 48 |
| `stub:imm32!ImmGetConversionStatus` | 23 | 68 |
| `stub:dwmapi!DwmSetWindowAttribute` | 22 | 90 |
| `stub:imm32!ImmSetOpenStatus` | 17 | 44 |
| `stub:kernel32!AttachConsole` | 12 | 12 |
| `stub:kernel32!SetConsoleOutputCP` | 11 | 11 |
| `stub:imm32!ImmIsIME` | 10 | 33 |
| `stub:kernel32!SetConsoleCP` | 10 | 10 |
| `stub:dwmapi!DwmEnableComposition` | 9 | 9 |
| `stub:kernel32!GetConsoleScreenBufferInfo` | 8 | 53 |
| `stub:ole32!CoGetApartmentType` | 8 | 19 |
| `stub:uxtheme!RefreshImmersiveColorPolicyState` | 8 | 18 |
| `stub:uxtheme!SetPreferredAppMode` | 8 | 18 |
| `stub:user32!SwapMouseButton` | 8 | 16 |
| `stub:avifil32!AVIFileInit` | 8 | 9 |
| `stub:kernel32!SetConsoleMode` | 8 | 8 |
| `stub:user32!GetThreadDesktop` | 8 | 8 |
| `stub:setupapi!SetupDiEnumDeviceInterfaces` | 7 | 5120 |
| `stub:user32!IsTouchWindow` | 7 | 24 |
| `stub:imm32!ImmSetCompositionFontA` | 7 | 11 |
| `stub:kernel32!SetConsoleTextAttribute` | 6 | 68 |
| `stub:winmm!waveInGetNumDevs` | 6 | 42 |
| `stub:uxtheme!ShouldAppsUseDarkMode` | 6 | 18 |
| `stub:imm32!ImmCreateContext` | 6 | 6 |
| `stub:kernel32!GetConsoleProcessList` | 6 | 6 |
| `stub:kernel32!PowerCreateRequest` | 6 | 6 |
| `stub:kernel32!PowerSetRequest` | 6 | 6 |
| `stub:shell32!ExtractIconExA` | 6 | 6 |
| `stub:shell32!SetCurrentProcessExplicitAppUserModelID` | 6 | 6 |
| `stub:uiautomationcore!UiaLookupId` | 6 | 6 |
| `stub:user32!RegisterPowerSettingNotification` | 6 | 6 |
| `stub:uxtheme!FlushMenuThemes` | 6 | 6 |
| `stub:user32!GetClipCursor` | 5 | 63302 |
| `stub:winhttp!WinHttpGetIEProxyConfigForCurrentUser` | 5 | 11 |
| `stub:crypt32!CertOpenSystemStoreA` | 5 | 5 |
| `stub:imm32!ImmDisableTextFrameService` | 5 | 5 |
| `glmissing:glDrawBuffers` | 4 | 1008 |
| `stub:imm32!ImmNotifyIME` | 4 | 4 |
| `stub:shell32!InitNetworkAddressControl` | 4 | 4 |
| `stub:user32!SetGestureConfig` | 4 | 4 |
| `stub:winmm!joyReleaseCapture` | 4 | 4 |
| `stub:user32!GetCursorInfo` | 3 | 225007 |
| `stub:shell32!ExtractIconA` | 3 | 5 |
| `stub:ucrtbase!iswxdigit` | 2 | 5105 |
| `stub:imm32!ImmAssociateContextEx` | 2 | 113 |
| `stub:ucrtbase!iswalnum` | 2 | 63 |
| `stub:ucrtbase!logbf` | 2 | 5 |
| `stub:user32!SetMenuInfo` | 2 | 4 |
| `stub:crypt32!CertOpenSystemStoreW` | 2 | 2 |
| `stub:imm32!ImmGetProperty` | 2 | 2 |
| `stub:kernel32!CreateMemoryResourceNotification` | 2 | 2 |
| `stub:kernel32!GetConsoleWindow` | 2 | 2 |
| `stub:kernel32!GetGeoInfoW` | 2 | 2 |
| `stub:kernel32!GetUserGeoID` | 2 | 2 |
