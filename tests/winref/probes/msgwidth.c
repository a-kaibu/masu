/* WPARAM, LPARAM, LRESULT and LONG_PTR at their width (review 6852be1f: sake kept them as 32-bit
 * values, PostMessage's wParam 0x1122334455667788 came back as 0x55667788 on x64, and
 * Get/SetWindowLongPtr were the 32-bit functions). Through the real entry points: PostMessage /
 * PostThreadMessage then GetMessage / DispatchMessage, SendMessage from this thread and from
 * another, SendMessageTimeout's DWORD_PTR result, CallWindowProc, a TIMERPROC's idEvent and
 * SetTimer's answer, a dialog's WM_INITDIALOG lParam and DialogBox's INT_PTR, and the window and
 * class longs (GWLP_USERDATA, GWLP_ID, extra bytes 8 at a time on x64, the 32-bit functions'
 * halves). The values are the same on both architectures; x86 keeps their low 32 bits. */
#include "../probe.h"

#define BIGW ((WPARAM)0x1122334455667788ull)
#define BIGL ((LPARAM)0x8877665544332211ull)
#define BIGR ((LRESULT)0x0123456789abcdefull)

static void hex(const char *k, unsigned long long v) {
    printf("%s %08lx%08lx\n", k, (unsigned long)(v >> 32), (unsigned long)v);
}

static WPARAM seen_w;
static LPARAM seen_l;
static UINT seen_m;

static LRESULT CALLBACK proc(HWND h, UINT m, WPARAM w, LPARAM l) {
    if (m >= WM_APP && m < WM_APP + 16) {
        seen_m = m, seen_w = w, seen_l = l;
        return m == WM_APP + 2 ? BIGR : (LRESULT)-2;
    }
    return DefWindowProcA(h, m, w, l);
}

static void seen(const char *what) {
    printf("%s: proc got %x\n", what, seen_m - WM_APP);
    hex("  wParam", (unsigned long long)seen_w);
    hex("  lParam", (unsigned long long)(LONG_PTR)seen_l);
    seen_m = 0, seen_w = 0, seen_l = 0;
}

static void get(const char *what, HWND h) {
    MSG msg;
    memset(&msg, 0, sizeof msg);
    BOOL ok = GetMessageA(&msg, h, WM_APP, WM_APP + 15);
    printf("%s: GetMessage %d message %x\n", what, ok, msg.message - WM_APP);
    hex("  wParam", (unsigned long long)msg.wParam);
    hex("  lParam", (unsigned long long)(LONG_PTR)msg.lParam);
    if (msg.hwnd) {
        LRESULT r = DispatchMessageA(&msg);
        hex("  DispatchMessage", (unsigned long long)r);
        seen("  dispatched");
    }
}

static HWND target;
static LRESULT other_result;
static DWORD WINAPI sender(void *p) {
    (void)p;
    other_result = SendMessageA(target, WM_APP + 2, BIGW, BIGL);
    return 0;
}

static UINT_PTR timer_id;
static int timer_calls;
static VOID CALLBACK on_timer(HWND h, UINT m, UINT_PTR id, DWORD t) {
    (void)h, (void)t;
    if (m == WM_TIMER && !timer_calls++) timer_id = id;
}

static INT_PTR CALLBACK dlgproc(HWND h, UINT m, WPARAM w, LPARAM l) {
    (void)w;
    if (m == WM_INITDIALOG) {
        hex("dialog WM_INITDIALOG lParam", (unsigned long long)(LONG_PTR)l);
        EndDialog(h, (INT_PTR)0x7edcba9876543210ull);
        return TRUE;
    }
    return FALSE;
}

int main(void) {
    WNDCLASSA wc = { 0 };
    wc.lpfnWndProc = proc;
    wc.hInstance = GetModuleHandleA(NULL);
    wc.lpszClassName = "msgwidth";
    wc.cbClsExtra = 16;
    wc.cbWndExtra = 16;
    if (!RegisterClassA(&wc)) {
        printf("RegisterClass failed %lu\n", GetLastError());
        return 1;
    }
    HWND h = CreateWindowExA(0, "msgwidth", "msgwidth", WS_OVERLAPPEDWINDOW, 0, 0, 100, 100, NULL, NULL, wc.hInstance, NULL);
    target = h;

    // posted: the queue keeps both parameters whole
    printf("PostMessage %d\n", PostMessageA(h, WM_APP + 1, BIGW, BIGL));
    get("posted", h);
    printf("PostThreadMessage %d\n", PostThreadMessageA(GetCurrentThreadId(), WM_APP + 3, BIGW, (LPARAM)-5));
    get("thread", NULL);

    // sent: the procedure's arguments and its LRESULT
    LRESULT r = SendMessageA(h, WM_APP + 2, BIGW, BIGL);
    hex("SendMessage", (unsigned long long)r);
    seen("sent");
    DWORD_PTR res = 0;
    LRESULT ok = SendMessageTimeoutA(h, WM_APP + 2, BIGW, BIGL, SMTO_NORMAL, 1000, &res);
    printf("SendMessageTimeout %d\n", ok != 0);
    hex("  result", (unsigned long long)res);
    seen("timeout");
    r = CallWindowProcA(proc, h, WM_APP + 2, BIGW, BIGL);
    hex("CallWindowProc", (unsigned long long)r);
    seen("called");

    // sent from another thread: the receiver pumps while it waits
    HANDLE t = CreateThread(NULL, 0, sender, NULL, 0, NULL);
    while (WaitForSingleObject(t, 0) == WAIT_TIMEOUT) {
        MSG msg;
        MsgWaitForMultipleObjects(1, &t, FALSE, 100, QS_ALLINPUT);
        while (PeekMessageA(&msg, NULL, 0, 0, PM_REMOVE)) DispatchMessageA(&msg);
    }
    CloseHandle(t);
    hex("other thread's SendMessage", (unsigned long long)other_result);
    seen("from another thread");

    // a timer: the id SetTimer answers and the one its TIMERPROC gets
    UINT_PTR id = SetTimer(h, (UINT_PTR)0x123456789ull, 1, on_timer);
    hex("SetTimer", (unsigned long long)id);
    DWORD end = GetTickCount() + 2000;
    while (!timer_calls && (int)(end - GetTickCount()) > 0) {
        MSG msg;
        if (PeekMessageA(&msg, NULL, 0, 0, PM_REMOVE)) DispatchMessageA(&msg);
        else Sleep(1);
    }
    hex("TIMERPROC idEvent", (unsigned long long)timer_id);
    printf("KillTimer %d\n", KillTimer(h, id));

    // window longs
    SetLastError(0);
    LONG_PTR old = SetWindowLongPtrA(h, GWLP_USERDATA, (LONG_PTR)0x0fedcba987654321ull);
    hex("SetWindowLongPtr USERDATA old", (unsigned long long)old);
    hex("GetWindowLongPtr USERDATA", (unsigned long long)GetWindowLongPtrA(h, GWLP_USERDATA));
    hex("GetWindowLong USERDATA", (unsigned long long)(LONG_PTR)GetWindowLongA(h, GWLP_USERDATA));
    SetWindowLongA(h, GWLP_USERDATA, (LONG)0x80000001);
    hex("after SetWindowLong USERDATA", (unsigned long long)GetWindowLongPtrA(h, GWLP_USERDATA));
    SetWindowLongPtrA(h, 0, (LONG_PTR)0x1122334455667788ull);
    hex("GetWindowLongPtr 0", (unsigned long long)GetWindowLongPtrA(h, 0));
    hex("GetWindowLong 0", (unsigned long long)(LONG_PTR)GetWindowLongA(h, 0));
    hex("GetWindowLong 4", (unsigned long long)(LONG_PTR)GetWindowLongA(h, 4));
    SetLastError(0);
    old = SetWindowLongPtrA(h, 12, (LONG_PTR)-1);
    printf("SetWindowLongPtr 12 of 16: error %lu\n", GetLastError());
    hex("  old", (unsigned long long)old);
    hex("GetWindowLong 12", (unsigned long long)(LONG_PTR)GetWindowLongA(h, 12));

    // class longs
    SetLastError(0);
    old = SetClassLongPtrA(h, 0, (LONG_PTR)0x0102030405060708ull);
    printf("SetClassLongPtr 0: error %lu\n", GetLastError());
    hex("GetClassLongPtr 0", (unsigned long long)GetClassLongPtrA(h, 0));
    hex("GetClassLong 4", (unsigned long long)(LONG_PTR)GetClassLongA(h, 4));
    SetLastError(0);
    GetClassLongPtrA(h, 16);
    printf("GetClassLongPtr 16 of 16: error %lu\n", GetLastError());

    // a child's ID is a LONG_PTR
    HWND ch = CreateWindowExA(0, "msgwidth", "", WS_CHILD, 0, 0, 10, 10, h, (HMENU)(ULONG_PTR)0x123400005678ull, wc.hInstance, NULL);
    hex("GWLP_ID", (unsigned long long)GetWindowLongPtrA(ch, GWLP_ID));
    printf("GetDlgCtrlID %x\n", GetDlgCtrlID(ch));
    DestroyWindow(ch);

    // a dialog: its init parameter and EndDialog's result
    struct { DLGTEMPLATE t; WORD menu, cls, title; } tpl;
    memset(&tpl, 0, sizeof tpl);
    tpl.t.style = WS_POPUP | DS_MODALFRAME;
    tpl.t.cx = 50, tpl.t.cy = 20;
    INT_PTR d = DialogBoxIndirectParamA(wc.hInstance, &tpl.t, h, dlgproc, (LPARAM)0x8000000000000005ull);
    hex("DialogBoxIndirectParam", (unsigned long long)d);

    DestroyWindow(h);
    return 0;
}
