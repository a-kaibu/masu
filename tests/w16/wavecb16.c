/* CALLBACK_FUNCTION delivers Win16 WOM_OPEN/DONE/CLOSE with far wave headers.
 * The owner receives a completion both during guest code and while waiting for a message.
 * libs: mmsystem.lib */
#include <windows.h>
#include <mmsystem.h>
#include <stdio.h>
#include <string.h>

#define N 1102
#define NOTIFY (WM_USER + 9)
#define INSTANCE 0x0123abcdUL
static char far buffers[3][N];
static WAVEHDR far headers[3];
static HWAVEOUT hwo, first_handle;
static HWND window;
static volatile unsigned count, done, good;
static unsigned sequence[5];

void FAR PASCAL __export Callback(HWAVEOUT dev, UINT msg, DWORD cookie, DWORD p1, DWORD p2) {
    if (count < 5) sequence[count] = msg;
    if (msg == MM_WOM_OPEN) first_handle = dev;
    if (dev != first_handle || cookie != INSTANCE || p2 != 0) good = 0;
    if (msg == MM_WOM_DONE) {
        if (done >= 3 || (LPWAVEHDR)p1 != &headers[done] ||
            (headers[done].dwFlags & (WHDR_DONE | WHDR_INQUEUE)) != WHDR_DONE) good = 0;
        ++done;
        if (done == 2) PostMessage(window, NOTIFY, 0, 0);
    } else if (p1 != 0) {
        good = 0;
    }
    ++count;
}

LONG FAR PASCAL __export WindowProc(HWND w, UINT m, WPARAM wp, LPARAM lp) {
    return DefWindowProc(w, m, wp, lp);
}

int PASCAL WinMain(HINSTANCE hi, HINSTANCE hp, LPSTR cmd, int sh) {
    WNDCLASS wc;
    PCMWAVEFORMAT fmt;
    MSG msg;
    UINT result, i;
    int woke = 0;
    memset(&wc, 0, sizeof wc);
    wc.hInstance = hi;
    wc.lpfnWndProc = WindowProc;
    wc.lpszClassName = "wavecb16";
    RegisterClass(&wc);
    window = CreateWindow(wc.lpszClassName, "", WS_POPUP, 0, 0, 20, 20, 0, 0, hi, 0);

    fmt.wf.wFormatTag = WAVE_FORMAT_PCM;
    fmt.wf.nChannels = 1;
    fmt.wf.nSamplesPerSec = 11025;
    fmt.wf.nAvgBytesPerSec = 11025;
    fmt.wf.nBlockAlign = 1;
    fmt.wBitsPerSample = 8;
    good = 1;
    result = waveOutOpen(&hwo, WAVE_MAPPER, (LPWAVEFORMAT)&fmt, (DWORD)Callback, INSTANCE, CALLBACK_FUNCTION);
    printf("open %u callback %d handle %d\n", result, count == 1, first_handle != 0 && first_handle == hwo);
    if (result || count != 1) return 1;
    for (i = 0; i < 3; ++i) {
        memset(buffers[i], 0x80, N);
        headers[i].lpData = buffers[i];
        headers[i].dwBufferLength = N;
        if (waveOutPrepareHeader(hwo, &headers[i], sizeof(WAVEHDR))) return 2;
    }
    if (waveOutWrite(hwo, &headers[0], sizeof(WAVEHDR))) return 3;
    while (!done) {}
    printf("callback during guest code %d\n", done == 1);

    SetTimer(window, 1, 5000, 0);
    if (waveOutWrite(hwo, &headers[1], sizeof(WAVEHDR))) return 4;
    while (GetMessage(&msg, 0, 0, 0)) {
        if (msg.message == NOTIFY) { woke = 1; break; }
        if (msg.message == WM_TIMER) break;
        DispatchMessage(&msg);
    }
    KillTimer(window, 1);
    printf("callback while waiting %d\n", woke && done == 2);

    if (waveOutPause(hwo) || waveOutWrite(hwo, &headers[2], sizeof(WAVEHDR)) || waveOutReset(hwo)) return 5;
    printf("reset returned header %d\n", done == 3);
    result = waveOutClose(hwo);
    printf("close %u ordered %d arguments %d\n", result,
        count == 5 && sequence[0] == MM_WOM_OPEN && sequence[1] == MM_WOM_DONE &&
        sequence[2] == MM_WOM_DONE && sequence[3] == MM_WOM_DONE && sequence[4] == MM_WOM_CLOSE, good != 0);
    DestroyWindow(window);
    return 0;
}
