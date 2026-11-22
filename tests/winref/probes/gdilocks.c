/* GDI and USER32 from several threads at once (review 6852be1f: sake's BitBlt from a window DC
 * found the source's window with the destination's surface locked, while SetWindowPos resized
 * a surface with USER32's lock held, so the threads hung; two blits between two windows, one
 * each way, locked the surfaces in opposite orders). Two threads blit between two top-level
 * windows, one each way, and a third resizes both, for a fixed number of rounds; then the
 * pixels of a known blit. Only what does not depend on scheduling is printed.
 * libs: -lgdi32 */
#include "../probe.h"

#define ROUNDS 300

static HWND w1, w2;
static volatile LONG done;

static DWORD WINAPI blitter(LPVOID p) {
    HWND from = p ? w1 : w2, to = p ? w2 : w1;
    for (int i = 0; i < ROUNDS; i++) {
        HDC s = GetDC(from), d = GetDC(to);
        BitBlt(d, i % 7, i % 5, 40, 30, s, 3, 2, SRCCOPY);
        StretchBlt(d, 0, 0, 20, 20, s, 0, 0, 40, 40, SRCCOPY);
        ReleaseDC(to, d);
        ReleaseDC(from, s);
    }
    InterlockedIncrement(&done);
    return 0;
}

static DWORD WINAPI resizer(LPVOID p) {
    (void)p;
    for (int i = 0; i < ROUNDS; i++) {
        SetWindowPos(w1, NULL, 0, 0, 120 + i % 40, 90 + i % 30, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        SetWindowPos(w2, NULL, 0, 0, 130 + i % 20, 100 + i % 10, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
    InterlockedIncrement(&done);
    return 0;
}

int main(void) {
    WNDCLASSA wc = { 0 };
    wc.lpfnWndProc = DefWindowProcA;
    wc.hInstance = GetModuleHandleA(NULL);
    wc.lpszClassName = "gdilocks";
    wc.hbrBackground = (HBRUSH)GetStockObject(WHITE_BRUSH);
    RegisterClassA(&wc);
    w1 = CreateWindowExA(0, "gdilocks", "one", WS_POPUP | WS_VISIBLE, 0, 0, 160, 120, NULL, NULL, wc.hInstance, NULL);
    w2 = CreateWindowExA(0, "gdilocks", "two", WS_POPUP | WS_VISIBLE, 200, 0, 160, 120, NULL, NULL, wc.hInstance, NULL);
    HANDLE t[3] = {
        CreateThread(NULL, 0, blitter, (LPVOID)1, 0, NULL),
        CreateThread(NULL, 0, blitter, NULL, 0, NULL),
        CreateThread(NULL, 0, resizer, NULL, 0, NULL),
    };
    // (the windows belong to this thread: it pumps while the others work)
    while (WaitForMultipleObjects(3, t, TRUE, 0) == WAIT_TIMEOUT) {
        MSG m;
        MsgWaitForMultipleObjects(3, t, TRUE, 50, QS_ALLINPUT);
        while (PeekMessageA(&m, NULL, 0, 0, PM_REMOVE)) DispatchMessageA(&m);
    }
    printf("threads done: %ld of 3\n", done);

    // a known blit once the threads are done: red into w1, copied into w2, read back
    SetWindowPos(w1, NULL, 0, 0, 160, 120, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    SetWindowPos(w2, NULL, 0, 0, 160, 120, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    HDC a = GetDC(w1), b = GetDC(w2);
    RECT r = { 10, 10, 30, 20 };
    HBRUSH red = CreateSolidBrush(RGB(255, 0, 0));
    FillRect(a, &r, red);
    printf("BitBlt %d\n", BitBlt(b, 50, 40, 20, 10, a, 10, 10, SRCCOPY));
    printf("pixel in %06lx, beside %06lx\n", (unsigned long)GetPixel(b, 55, 45), (unsigned long)GetPixel(b, 75, 45));
    ReleaseDC(w1, a);
    ReleaseDC(w2, b);
    DeleteObject(red);
    DestroyWindow(w1);
    DestroyWindow(w2);
    return 0;
}
