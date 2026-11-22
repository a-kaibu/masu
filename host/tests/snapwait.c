/* Save states in the middle of timed waits (review 6852be1f section 7: a wait's deadline is
 * the state's data, so a restored copy waits what was left, not the whole time again). The
 * main thread sleeps, then waits on an event no one sets, then on a semaphore a worker releases
 * after its own Sleep; tests/snap.sh cuts each wait in the middle and compares the output
 * with the whole run's. Each line says whether the wait took its time (GetTickCount, whose
 * clock goes on from the state's), not the time itself. */
#include <windows.h>
#include <stdio.h>

#define MS 2500
/* (a wait is on time when it ends within this of its deadline: the restored copy's start, a
 * slow runner) */
#define SLACK 900

static HANDLE sem;

static DWORD WINAPI worker(LPVOID p) {
    (void)p;
    Sleep(MS);
    ReleaseSemaphore(sem, 1, NULL);
    return 0;
}

static void report(const char *what, DWORD r, DWORD want_r, DWORD took) {
    printf("%s: %s, %s\n", what, r == want_r ? "answer ok" : "wrong answer", took >= MS - 20 && took < MS + SLACK ? "on time" : took < MS - 20 ? "early" : "late");
    fflush(stdout);
}

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    printf("start\n");
    DWORD t = GetTickCount();
    Sleep(MS);
    report("Sleep", 0, 0, GetTickCount() - t);

    HANDLE ev = CreateEventA(NULL, TRUE, FALSE, NULL);
    t = GetTickCount();
    DWORD r = WaitForSingleObject(ev, MS);
    report("WaitForSingleObject", r, WAIT_TIMEOUT, GetTickCount() - t);

    sem = CreateSemaphoreA(NULL, 0, 1, NULL);
    HANDLE w = CreateThread(NULL, 0, worker, NULL, 0, NULL);
    t = GetTickCount();
    // (signalled by the worker at MS, long before this wait's own timeout)
    r = WaitForSingleObject(sem, 4 * MS);
    report("signalled", r, WAIT_OBJECT_0, GetTickCount() - t);
    WaitForSingleObject(w, INFINITE);
    printf("end\n");
    return 0;
}
