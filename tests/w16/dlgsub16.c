/* Dialog subclassing changes the DLGPROC while ordinary window extra data stays a LONG. */
#include <windows.h>
#include <stdio.h>
#include <string.h>

#define TEST_MSG (WM_USER + 11)
static DLGPROC previous;

BOOL FAR PASCAL __export Original(HWND w, UINT m, WPARAM wp, LPARAM lp);

BOOL FAR PASCAL __export Replacement(HWND w, UINT m, WPARAM wp, LPARAM lp) {
    if (m != TEST_MSG) return FALSE;
    printf("replacement gets %u %ld\n", wp, lp);
    return (BOOL)CallWindowProc((FARPROC)previous, w, m, wp + 1, lp);
}

BOOL FAR PASCAL __export Original(HWND w, UINT m, WPARAM wp, LPARAM lp) {
    if (m == TEST_MSG) {
        SetWindowLong(w, DWL_MSGRESULT, lp + wp);
        return TRUE;
    }
    if (m == WM_INITDIALOG) {
        DLGPROC replaced;
        LONG delivered, original;
        printf("initial procedure %d\n", (DLGPROC)GetWindowLong(w, DWL_DLGPROC) == Original);
        previous = (DLGPROC)SetWindowLong(w, DWL_DLGPROC, (LONG)Replacement);
        printf("old procedure %d, new procedure %d\n", previous == Original,
            (DLGPROC)GetWindowLong(w, DWL_DLGPROC) == Replacement);
        delivered = SendMessage(w, TEST_MSG, 7, 200);
        replaced = (DLGPROC)SetWindowLong(w, DWL_DLGPROC, (LONG)previous);
        original = SendMessage(w, TEST_MSG, 7, 200);
        printf("messages %ld %ld, restored %d\n", delivered, original, replaced == Replacement);
        EndDialog(w, 23);
        return TRUE;
    }
    return FALSE;
}

LONG FAR PASCAL __export WindowProc(HWND w, UINT m, WPARAM wp, LPARAM lp) {
    return DefWindowProc(w, m, wp, lp);
}

int PASCAL WinMain(HINSTANCE hi, HINSTANCE hp, LPSTR cmd, int sh) {
    WNDCLASS wc;
    HWND w;
    int result;
    memset(&wc, 0, sizeof wc);
    wc.lpfnWndProc = WindowProc;
    wc.cbWndExtra = 8;
    wc.hInstance = hi;
    wc.lpszClassName = "plain16";
    RegisterClass(&wc);
    w = CreateWindow("plain16", "", WS_POPUP, 0, 0, 40, 30, 0, 0, hi, 0);
    SetWindowLong(w, DWL_DLGPROC, 0x12345678L);
    printf("window extra %08lx\n", GetWindowLong(w, DWL_DLGPROC));
    DestroyWindow(w);
    result = DialogBox(hi, MAKEINTRESOURCE(1), 0, (DLGPROC)Original);
    printf("dialog result %d\n", result);
    return 0;
}
