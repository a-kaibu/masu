# Use-case coverage

344 game runs, 3959 entry points they call, 6174 the tests call.

| kind | used by games | tested | by games weighted | by calls weighted |
|---|---|---|---|---|
| IAMStreamSelect | 3 | 1 | 33.3% | 25.0% |
| IBaseFilter | 9 | 9 | 100.0% | 100.0% |
| IEnumPins | 2 | 2 | 100.0% | 100.0% |
| IMediaSample | 13 | 0 | 0.0% | 0.0% |
| IMemAllocator | 4 | 0 | 0.0% | 0.0% |
| IMemInputPin | 5 | 0 | 0.0% | 0.0% |
| IPin | 11 | 8 | 83.9% | 98.9% |
| api | 3377 | 1550 | 79.2% | 87.8% |
| d3d | 258 | 136 | 66.6% | 84.2% |
| d3d9 | 141 | 111 | 86.3% | 98.5% |
| glmissing | 1 | 0 | 0.0% | 0.0% |
| stub | 135 | 45 | 40.7% | 0.5% |

## Called by games, reached by no test (2006)

| entry point | games | calls |
|---|---|---|
| `api:kernel32!InitializeCriticalSectionAndSpinCount` | 224 | 8140 |
| `api:kernel32!SetHandleCount` | 215 | 482 |
| `api:sake!IDirectSoundBuffer8::15` | 201 | 129684 |
| `api:kernel32!EncodePointer` | 196 | 144413 |
| `api:user32!SetCursor` | 150 | 20683 |
| `api:kernel32!DecodePointer` | 140 | 149515 |
| `api:sake!d3d9:2:57` | 103 | 3222174 |
| `api:sake!d3d9:2:43` | 101 | 133330 |
| `api:sake!d3d9:1:8` | 101 | 1453 |
| `d3d9:IDirect3D9::GetAdapterDisplayMode` | 101 | 1453 |
| `api:sake!d3d9:2:65` | 100 | 2527397 |
| `api:sake!d3d9:2:69` | 100 | 1908631 |
| `api:sake!d3d9:2:17` | 100 | 75927 |
| `api:sake!IDirectSoundBuffer8::16` | 97 | 1494 |
| `api:sake!d3d9:5:19` | 96 | 32547 |
| `api:sake!d3d9:5:20` | 96 | 32513 |
| `api:sake!d3d9:2:67` | 87 | 1657377 |
| `api:sake!d3d9:2:89` | 86 | 514765 |
| `api:sake!d3d9:1:14` | 79 | 130 |
| `api:user32!RegisterWindowMessageA` | 75 | 233 |
| `api:shell32!DragAcceptFiles` | 70 | 80 |
| `api:sake!d3d9:2:47` | 69 | 1178703 |
| `api:kernel32!InitializeSRWLock` | 67 | 32213 |
| `api:user32!ShowCursor` | 67 | 488 |
| `api:sake!d3d11:70:5` | 64 | 5878 |
| `api:sake!d3d11:70:3` | 63 | 6878 |
| `api:kernel32!LocaleNameToLCID` | 63 | 4477 |
| `api:sake!d3d11:70:7` | 63 | 4080 |
| `api:sake!d3d11:70:0` | 63 | 266 |
| `api:sake!d3d11:74:2` | 62 | 880799 |
| `api:sake!d3d11:71:44` | 62 | 58712 |
| `api:sake!d3d11:70:20` | 62 | 708 |
| `api:sake!d3d11:71:14` | 61 | 398144 |
| `api:user32!PtInRect` | 60 | 436568 |
| `api:sake!d3d11:71:15` | 60 | 398120 |
| `api:sake!d3d11:71:48` | 60 | 383787 |
| `api:sake!d3d11:71:9` | 60 | 139821 |
| `api:sake!d3d11:71:11` | 60 | 113040 |
| `api:sake!d3d11:71:8` | 59 | 919934 |
| `api:kernel32!InitializeConditionVariable` | 59 | 972 |
| `api:user32!WINNLSEnableIME` | 56 | 189 |
| `api:sake!d3d11:64:7` | 56 | 141 |
| `api:sake!d3d11:70:29` | 54 | 3904 |
| `d3d:70.29` | 54 | 3904 |
| `api:sake!d3d9:2:18` | 51 | 35089 |
| `api:sake!d3d11:70:6` | 51 | 114 |
| `api:sake!d3d11:71:23` | 50 | 63527 |
| `d3d:71.23` | 50 | 63526 |
| `api:sake!d3d9:1:5` | 50 | 82 |
| `d3d9:IDirect3D9::GetAdapterIdentifier` | 50 | 82 |
| `api:sake!d3d11:68:2` | 50 | 54 |
| `d3d:68.2` | 50 | 54 |
| `api:kernel32!GetLocaleInfoEx` | 48 | 785 |
| `api:sake!d3d9:5:17` | 47 | 146572 |
| `d3d9:Texture::GetLevelDesc` | 47 | 146562 |
| `api:sake!d3d11:71:0` | 47 | 89 |
| `api:sake!d3d11:64:0` | 47 | 87 |
| `api:sake!d3d11:70:24` | 46 | 17872 |
| `api:sake!d3d11:91:2` | 46 | 10530 |
| `api:sake!d3d11:70:33` | 46 | 6239 |

## Called by games, not implemented (136)

| entry point | games | calls |
|---|---|---|
| `stub:imm32!ImmGetContext` | 90 | 779 |
| `stub:imm32!ImmAssociateContext` | 74 | 87 |
| `stub:kernel32!InitializeProcThreadAttributeList` | 38 | 76 |
| `stub:kernel32!DeleteProcThreadAttributeList` | 38 | 38 |
| `stub:imm32!ImmReleaseContext` | 35 | 115 |
| `stub:user32!RegisterTouchWindow` | 35 | 35 |
| `stub:imm32!ImmGetOpenStatus` | 25 | 52 |
| `stub:imm32!ImmGetConversionStatus` | 23 | 72 |
| `stub:dwmapi!DwmSetWindowAttribute` | 22 | 93 |
| `stub:imm32!ImmSetOpenStatus` | 17 | 44 |
| `stub:kernel32!AttachConsole` | 12 | 12 |
| `stub:kernel32!SetConsoleOutputCP` | 11 | 11 |
| `stub:imm32!ImmIsIME` | 10 | 33 |
| `stub:kernel32!SetConsoleCP` | 10 | 10 |
| `stub:dwmapi!DwmEnableComposition` | 9 | 9 |
| `stub:setupapi!SetupDiEnumDeviceInterfaces` | 8 | 5633 |
| `stub:kernel32!GetConsoleScreenBufferInfo` | 8 | 66 |
| `stub:ole32!CoGetApartmentType` | 8 | 19 |
| `stub:uxtheme!RefreshImmersiveColorPolicyState` | 8 | 18 |
| `stub:uxtheme!SetPreferredAppMode` | 8 | 18 |
| `stub:user32!SwapMouseButton` | 8 | 16 |
| `stub:avifil32!AVIFileInit` | 8 | 9 |
| `stub:kernel32!SetConsoleMode` | 8 | 8 |
| `stub:user32!GetThreadDesktop` | 8 | 8 |
| `stub:user32!IsTouchWindow` | 7 | 24 |
| `stub:imm32!ImmSetCompositionFontA` | 7 | 11 |
| `stub:kernel32!SetConsoleTextAttribute` | 6 | 72 |
| `stub:winmm!waveInGetNumDevs` | 6 | 46 |
| `stub:uxtheme!ShouldAppsUseDarkMode` | 6 | 18 |
| `stub:crypt32!CertOpenSystemStoreA` | 6 | 6 |
| `stub:imm32!ImmCreateContext` | 6 | 6 |
| `stub:kernel32!GetConsoleProcessList` | 6 | 6 |
| `stub:kernel32!PowerCreateRequest` | 6 | 6 |
| `stub:kernel32!PowerSetRequest` | 6 | 6 |
| `stub:shell32!ExtractIconExA` | 6 | 6 |
| `stub:shell32!SetCurrentProcessExplicitAppUserModelID` | 6 | 6 |
| `stub:uiautomationcore!UiaLookupId` | 6 | 6 |
| `stub:uxtheme!FlushMenuThemes` | 6 | 6 |
| `stub:user32!GetClipCursor` | 5 | 55138 |
| `stub:winhttp!WinHttpGetIEProxyConfigForCurrentUser` | 5 | 12 |
| `stub:imm32!ImmDisableTextFrameService` | 5 | 5 |
| `stub:user32!RegisterPowerSettingNotification` | 5 | 5 |
| `stub:user32!GetCursorInfo` | 4 | 207191 |
| `glmissing:glDrawBuffers` | 4 | 758 |
| `stub:imm32!ImmNotifyIME` | 4 | 4 |
| `stub:shell32!InitNetworkAddressControl` | 4 | 4 |
| `stub:user32!SetGestureConfig` | 4 | 4 |
| `stub:winmm!joyReleaseCapture` | 4 | 4 |
| `stub:shell32!ExtractIconA` | 3 | 5 |
| `stub:ucrtbase!iswxdigit` | 2 | 3611 |
| `stub:imm32!ImmAssociateContextEx` | 2 | 128 |
| `stub:ucrtbase!iswalnum` | 2 | 63 |
| `stub:imm32!ImmGetCompositionStringW` | 2 | 28 |
| `stub:ucrtbase!logbf` | 2 | 5 |
| `stub:user32!SetMenuInfo` | 2 | 4 |
| `stub:crypt32!CertOpenSystemStoreW` | 2 | 2 |
| `stub:imm32!ImmGetProperty` | 2 | 2 |
| `stub:kernel32!CreateMemoryResourceNotification` | 2 | 2 |
| `stub:kernel32!GetConsoleWindow` | 2 | 2 |
| `stub:kernel32!GetGeoInfoW` | 2 | 2 |
