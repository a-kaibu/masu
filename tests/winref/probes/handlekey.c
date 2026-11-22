/* A HANDLE is a handle, not the object (review 6852be1f: sake kept an object under the number of
 * its first handle and found it by that number after the handle closed, so a closed HANDLE still
 * reached an object a duplicate kept open; and a thread created suspended waited for
 * ResumeThread on CreateThread's handle number, so ResumeThread through a DuplicateHandle'd one
 * missed it and the thread never started). Through the real entry points: a closed first handle
 * with a duplicate alive (SetEvent, WaitForSingleObject, DuplicateHandle, GetHandleInformation,
 * CloseHandle), and a CREATE_SUSPENDED thread suspended, resumed and terminated through
 * duplicates, its first handle closed before it started. No handle is made between a close and
 * the calls on the closed number (Windows may hand the number out again). */
#include "../probe.h"

#define E(expr) do { SetLastError(0xdead); long long r_ = (long long)(expr); printf("%-64s = %lld err %lu\n", #expr, r_, GetLastError()); } while (0)

static volatile LONG ran;

static DWORD WINAPI worker(LPVOID p) {
    InterlockedIncrement(&ran);
    return (DWORD)(ULONG_PTR)p;
}

static HANDLE dup(HANDLE h) {
    HANDLE d = NULL;
    if (!DuplicateHandle(GetCurrentProcess(), h, GetCurrentProcess(), &d, 0, FALSE, DUPLICATE_SAME_ACCESS)) {
        printf("DuplicateHandle failed %lu\n", GetLastError());
    }
    return d;
}

int main(void) {
    HANDLE self = GetCurrentProcess();
    DWORD flags;
    HANDLE out;

    // an event: its first handle closed, a duplicate keeps it
    HANDLE ev = CreateEventA(NULL, TRUE, FALSE, NULL);
    HANDLE ev2 = dup(ev);
    E(CloseHandle(ev));
    E(SetEvent(ev));
    E(WaitForSingleObject(ev2, 0));
    E(ResetEvent(ev));
    E(WaitForSingleObject(ev, 0));
    E(GetHandleInformation(ev, &flags));
    E(DuplicateHandle(self, ev, self, &out, 0, FALSE, DUPLICATE_SAME_ACCESS));
    E(CloseHandle(ev));
    E(SetEvent(ev2));
    E(WaitForSingleObject(ev2, 0));
    E(CloseHandle(ev2));
    E(SetEvent(ev2));

    // a thread created suspended, resumed through a duplicate of its handle
    ran = 0;
    HANDLE t = CreateThread(NULL, 0, worker, (LPVOID)41, CREATE_SUSPENDED, NULL);
    HANDLE t2 = dup(t);
    Sleep(50);
    printf("ran before ResumeThread: %ld\n", ran);
    E(ResumeThread(t2));
    E(WaitForSingleObject(t, 2000));
    printf("ran: %ld\n", ran);
    DWORD code = 0;
    E(GetExitCodeThread(t2, &code));
    printf("exit code %lu\n", code);
    CloseHandle(t);
    CloseHandle(t2);

    // suspended twice through two handles, its first handle closed, resumed through the other
    ran = 0;
    t = CreateThread(NULL, 0, worker, (LPVOID)42, CREATE_SUSPENDED, NULL);
    t2 = dup(t);
    E(SuspendThread(t2));
    E(CloseHandle(t));
    E(ResumeThread(t));
    E(ResumeThread(t2));
    Sleep(50);
    printf("ran with count 1: %ld\n", ran);
    E(ResumeThread(t2));
    E(WaitForSingleObject(t2, 2000));
    printf("ran: %ld\n", ran);
    E(GetExitCodeThread(t2, &code));
    printf("exit code %lu\n", code);
    E(ResumeThread(t2));
    CloseHandle(t2);

    // terminated through a duplicate before it started
    ran = 0;
    t = CreateThread(NULL, 0, worker, (LPVOID)43, CREATE_SUSPENDED, NULL);
    t2 = dup(t);
    E(TerminateThread(t2, 77));
    E(WaitForSingleObject(t, 2000));
    E(GetExitCodeThread(t, &code));
    printf("exit code %lu, ran %ld\n", code, ran);
    CloseHandle(t);
    CloseHandle(t2);
    return 0;
}
