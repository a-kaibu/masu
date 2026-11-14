/* A thread suspended (SuspendThread) while it waits does not take what it waits for when that
 * comes free: it cannot run to. KiriKiri2 suspends its sound thread while that thread waits for
 * a critical section the main thread holds, lets the section go, and enters it again: sake let
 * the suspended waiter take it, the waiter stood still holding it, and the main thread waited
 * for ever (#221). The same for an auto-reset event and a mutex. The probe waits (Sleep) before
 * it looks: SuspendThread is asynchronous. */
#include "../probe.h"

#define E(expr) do { SetLastError(0xdead); long long r_ = (long long)(expr); printf("%-60s = %lld err %lu\n", #expr, r_, GetLastError()); } while (0)

static CRITICAL_SECTION cs;
static HANDLE ev, mtx;
static volatile LONG got_cs, got_ev, got_mtx;

static DWORD WINAPI cs_waiter(LPVOID p) {
    EnterCriticalSection(&cs);
    InterlockedIncrement(&got_cs);
    LeaveCriticalSection(&cs);
    return 1;
}
static DWORD WINAPI ev_waiter(LPVOID p) {
    DWORD r = WaitForSingleObject(ev, 10000);
    InterlockedIncrement(&got_ev);
    return 10 + r;
}
static DWORD WINAPI mtx_waiter(LPVOID p) {
    DWORD r = WaitForSingleObject(mtx, 10000);
    InterlockedIncrement(&got_mtx);
    if (r == WAIT_OBJECT_0) ReleaseMutex(mtx);
    return 20 + r;
}

int main(void) {
    DWORD code;
    InitializeCriticalSection(&cs);

    /* a critical section */
    EnterCriticalSection(&cs);
    HANDLE t = CreateThread(NULL, 0, cs_waiter, NULL, 0, NULL);
    Sleep(100);
    E(SuspendThread(t));
    Sleep(50);
    LeaveCriticalSection(&cs);
    Sleep(100);
    E(TryEnterCriticalSection(&cs)); /* the suspended waiter did not take it */
    printf("cs waiter got it while suspended: %ld\n", got_cs);
    LeaveCriticalSection(&cs);
    E(ResumeThread(t));
    E(WaitForSingleObject(t, 5000));
    E(GetExitCodeThread(t, &code)); printf("code %lu, got %ld\n", code, got_cs);

    /* the same twice in a row: entered again at once (KiriKiri2's loop) */
    EnterCriticalSection(&cs);
    t = CreateThread(NULL, 0, cs_waiter, NULL, 0, NULL);
    Sleep(100);
    E(SuspendThread(t));
    Sleep(50);
    LeaveCriticalSection(&cs);
    EnterCriticalSection(&cs);
    LeaveCriticalSection(&cs);
    Sleep(50);
    EnterCriticalSection(&cs);
    printf("entered again, cs waiter got it: %ld\n", got_cs);
    LeaveCriticalSection(&cs);
    E(ResumeThread(t));
    E(WaitForSingleObject(t, 5000));
    printf("got %ld\n", got_cs);

    /* an auto-reset event */
    ev = CreateEventA(NULL, FALSE, FALSE, NULL);
    t = CreateThread(NULL, 0, ev_waiter, NULL, 0, NULL);
    Sleep(100);
    E(SuspendThread(t));
    Sleep(50);
    E(SetEvent(ev));
    Sleep(100);
    E(WaitForSingleObject(ev, 0)); /* still set: the suspended waiter did not take it */
    printf("event waiter woke while suspended: %ld\n", got_ev);
    E(ResumeThread(t));
    Sleep(100);
    printf("event waiter after resume (still waiting): %ld\n", got_ev);
    E(SetEvent(ev));
    E(WaitForSingleObject(t, 5000));
    E(GetExitCodeThread(t, &code)); printf("code %lu, got %ld\n", code, got_ev);

    /* a mutex */
    mtx = CreateMutexA(NULL, TRUE, NULL);
    t = CreateThread(NULL, 0, mtx_waiter, NULL, 0, NULL);
    Sleep(100);
    E(SuspendThread(t));
    Sleep(50);
    E(ReleaseMutex(mtx));
    Sleep(100);
    E(WaitForSingleObject(mtx, 0)); /* free: the suspended waiter did not take it */
    printf("mutex waiter got it while suspended: %ld\n", got_mtx);
    E(ReleaseMutex(mtx));
    E(ResumeThread(t));
    E(WaitForSingleObject(t, 5000));
    E(GetExitCodeThread(t, &code)); printf("code %lu, got %ld\n", code, got_mtx);

    DeleteCriticalSection(&cs);
    return 0;
}
