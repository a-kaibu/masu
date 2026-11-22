/* The process ends with its last thread, not its first (review 6852be1f: sake ended the process
 * when the first thread ended, whatever else ran). The main thread starts two threads and calls
 * ExitThread; they go on (one waits for the other), print, and the last of them returns 7, which
 * is the process's exit code (run.sh records it). */
#include "../probe.h"

static HANDLE first, ready;

static DWORD WINAPI second_thread(LPVOID p) {
    (void)p;
    WaitForSingleObject(first, INFINITE);
    DWORD code = 0;
    GetExitCodeThread(first, &code);
    printf("second: the first worker ended with %lu, the main thread too\n", code);
    fflush(stdout);
    return 7;
}

static DWORD WINAPI first_thread(LPVOID p) {
    HANDLE main = (HANDLE)p;
    printf("first: waiting for the main thread\n");
    fflush(stdout);
    SetEvent(ready);
    DWORD w = WaitForSingleObject(main, 5000);
    DWORD code = 0;
    GetExitCodeThread(main, &code);
    printf("first: main thread ended %d with %lu\n", w == WAIT_OBJECT_0, code);
    fflush(stdout);
    Sleep(100);
    return 3;
}

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    HANDLE me;
    DuplicateHandle(GetCurrentProcess(), GetCurrentThread(), GetCurrentProcess(), &me, 0, FALSE, DUPLICATE_SAME_ACCESS);
    ready = CreateEventA(NULL, TRUE, FALSE, NULL);
    first = CreateThread(NULL, 0, first_thread, me, 0, NULL);
    CreateThread(NULL, 0, second_thread, NULL, 0, NULL);
    // (the lines in one order: the first thread's before the main thread's)
    WaitForSingleObject(ready, INFINITE);
    printf("main: ExitThread(5)\n");
    ExitThread(5);
}
