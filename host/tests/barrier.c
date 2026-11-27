#define _WIN32_WINNT 0x0602
#include <windows.h>
#include <stdio.h>

typedef struct Test {
    SYNCHRONIZATION_BARRIER barrier;
    HANDLE starting;
    BOOL returned_true;
} Test;

static DWORD WINAPI worker(void *opaque)
{
    Test *t = opaque;
    SetEvent(t->starting);
    t->returned_true = EnterSynchronizationBarrier(&t->barrier, 0);
    return 0;
}

static int phase(const char *name, Test *t)
{
    ResetEvent(t->starting);
    HANDLE thread = CreateThread(NULL, 0, worker, t, 0, NULL);
    if (!thread || WaitForSingleObject(t->starting, 1000) != WAIT_OBJECT_0) return 0;
    DWORD before = WaitForSingleObject(thread, 100);
    BOOL this_true = EnterSynchronizationBarrier(&t->barrier, 0);
    if (WaitForSingleObject(thread, 1000) != WAIT_OBJECT_0) return 0;
    CloseHandle(thread);
    printf("%s before %lu last arrivals %d\n", name, (unsigned long)before, !!this_true + !!t->returned_true);
    return 1;
}

int main(void)
{
    Test t;
    t.starting = CreateEventW(NULL, TRUE, FALSE, NULL);
    if (!t.starting || !InitializeSynchronizationBarrier(&t.barrier, 2, -1)) return 1;
    printf("delete before use %d\n", !!DeleteSynchronizationBarrier(&t.barrier));
    if (!phase("first", &t)) return 2;
    printf("delete after use %d\n", !!DeleteSynchronizationBarrier(&t.barrier));
    if (!phase("second", &t)) return 3;
    CloseHandle(t.starting);
    return 0;
}
