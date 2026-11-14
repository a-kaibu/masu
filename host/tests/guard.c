/* Page protection: PAGE_NOACCESS faults with STATUS_ACCESS_VIOLATION, PAGE_GUARD once with
 * STATUS_GUARD_PAGE_VIOLATION (then the page is plain), caught by a vectored handler that
 * fixes the page and continues. PAGE_READONLY: calls through a table of function pointers
 * there (a vtable in .rdata: `call [reg+reg*8]`, which also writes the stack) do not fault, a
 * store does. The accesses run in hot loops first, so that translated code
 * meets them as well as the interpreter. No C runtime; built for x86 and x86-64.
 * libs: -nostdlib -lkernel32 -luser32 */
#include <windows.h>

static void out(const char *f, ...)
{
    char b[512];
    DWORD n;
    va_list a;
    va_start(a, f);
    n = wvsprintfA(b, f, a);
    va_end(a);
    WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), b, n, &n, NULL);
}

static volatile DWORD code, faults;
static volatile char *seen;
static char *fix;

static LONG CALLBACK handler(EXCEPTION_POINTERS *e)
{
    DWORD old;
    code = e->ExceptionRecord->ExceptionCode;
    seen = (char *)e->ExceptionRecord->ExceptionInformation[1];
    faults++;
    if (code == EXCEPTION_ACCESS_VIOLATION && fix)
        VirtualProtect(fix, 4096, PAGE_READWRITE, &old);
    /* (a guard page is already plain again) */
    return code == EXCEPTION_ACCESS_VIOLATION || code == STATUS_GUARD_PAGE_VIOLATION ? EXCEPTION_CONTINUE_EXECUTION : EXCEPTION_CONTINUE_SEARCH;
}

/* sums n ints at p, every `step`-th; noinline so the same code runs for every page */
__attribute__((noinline)) static int sum(volatile int *p, int n, int step)
{
    int s = 0;
    for (int i = 0; i < n; i += step)
        s += p[i];
    return s;
}

__attribute__((noinline)) static void store(volatile int *p, int n, int v)
{
    for (int i = 0; i < n; i++)
        p[i] = v;
}

static unsigned f0(unsigned x) { return x + 1; }
static unsigned f1(unsigned x) { return x * 3; }
static unsigned f2(unsigned x) { return x ^ 0x55; }
static unsigned f3(unsigned x) { return x - 7; }
typedef unsigned (*fn)(unsigned);

/* calls through the table at t, in turn (noinline: the same code for every run) */
__attribute__((noinline)) static unsigned calls(fn *volatile t, int n)
{
    unsigned s = 0;
    fn *p = t;
    for (int i = 0; i < n; i++)
        s = p[i & 3](s);
    return s;
}

void mainCRTStartup(void)
{
    DWORD old;
    char *m = VirtualAlloc(NULL, 4 * 4096, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    AddVectoredExceptionHandler(1, handler);
    /* hot: page 0 a few thousand times */
    int s = 0;
    for (int k = 0; k < 4000; k++)
        s += sum((int *)m, 1024, 1);
    store((int *)(m + 4096), 1024, 7);

    /* page 1 no access: the read faults, the handler opens the page, the read is redone */
    VirtualProtect(m + 4096, 4096, PAGE_NOACCESS, &old);
    fix = m + 4096;
    faults = 0;
    s = sum((int *)(m + 4096), 1024, 1);
    out("noaccess read: faults %d code %x at +%x sum %d old %x\n", (int)faults, (unsigned)code,
        (unsigned)(seen - m), s, (unsigned)old);

    /* a store into it */
    VirtualProtect(m + 4096, 4096, PAGE_NOACCESS, &old);
    faults = 0;
    store((int *)(m + 4096), 1024, 3);
    out("noaccess write: faults %d code %x at +%x value %d\n", (int)faults, (unsigned)code,
        (unsigned)(seen - m), ((int *)(m + 4096))[1023]);
    fix = NULL;

    /* page 2 a guard page: one fault, then the page reads */
    VirtualProtect(m + 8192, 4096, PAGE_READWRITE | PAGE_GUARD, &old);
    MEMORY_BASIC_INFORMATION mbi;
    VirtualQuery(m + 8192, &mbi, sizeof mbi);
    out("guard query: protect %x\n", (unsigned)mbi.Protect);
    faults = 0;
    s = sum((int *)(m + 8192), 1024, 1);
    out("guard read: faults %d code %x at +%x sum %d\n", (int)faults, (unsigned)code, (unsigned)(seen - m), s);
    VirtualQuery(m + 8192, &mbi, sizeof mbi);
    out("guard after: protect %x\n", (unsigned)mbi.Protect);
    faults = 0;
    s = sum((int *)(m + 8192), 1024, 1);
    out("guard again: faults %d\n", (int)faults);

    /* page 3 read-only, a table of functions: called through, hot, without a fault */
    fn *t = (fn *)(m + 12288);
    t[0] = f0, t[1] = f1, t[2] = f2, t[3] = f3;
    VirtualProtect(m + 12288, 4096, PAGE_READONLY, &old);
    faults = 0;
    unsigned c = 0;
    for (int k = 0; k < 200; k++)
        c += calls(t, 1000 + k);
    out("readonly calls: faults %d sum %x\n", (int)faults, c);
    /* a store into it faults (the handler opens the page) */
    fix = m + 12288;
    faults = 0;
    store((int *)(m + 12288 + 64), 16, 5);
    out("readonly write: faults %d code %x at +%x value %d\n", (int)faults, (unsigned)code,
        (unsigned)(seen - m), ((int *)(m + 12288))[16 + 15]);
    fix = NULL;
    ExitProcess(0);
}
