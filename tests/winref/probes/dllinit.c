/* A DLL whose DllMain(PROCESS_ATTACH) answers FALSE (review 6852be1f: sake's LoadLibrary went on
 * and answered the module): LoadLibrary answers NULL with ERROR_DLL_INIT_FAILED, the DLL is told
 * PROCESS_DETACH at once and is gone, and the DLL it imports, attached before it, is detached
 * and unloaded with it. Twice (the second load starts afresh), then a DLL that answers TRUE the
 * second time it is asked. The same source builds the probe and its DLLs (build.sh, the line
 * below): dllinit-dep-NN.dll, and dllinit-fail-NN.dll importing from it.
 * dlls: fail dep */
#include "../probe.h"

#ifdef PROBE_DEP
__declspec(dllexport) int dep_value(void) { return 7; }
BOOL WINAPI DllMain(HINSTANCE h, DWORD why, LPVOID reserved) {
    if (why == DLL_PROCESS_ATTACH || why == DLL_PROCESS_DETACH) {
        printf("  dep %s\n", why ? "attach" : "detach");
        fflush(stdout);
    }
    return TRUE;
}
#elif defined PROBE_FAIL
__declspec(dllimport) int dep_value(void);
static int attaches;
__declspec(dllexport) int fail_value(void) { return dep_value() + attaches; }
BOOL WINAPI DllMain(HINSTANCE h, DWORD why, LPVOID reserved) {
    if (why == DLL_PROCESS_ATTACH || why == DLL_PROCESS_DETACH) {
        // (the environment carries what it should answer: a DLL's statics start afresh each load)
        char want[8] = "";
        GetEnvironmentVariableA("DLLINIT_ANSWER", want, sizeof want);
        if (why == DLL_PROCESS_ATTACH) attaches++;
        printf("  fail %s (answers %s)\n", why ? "attach" : "detach", why ? want : "-");
        fflush(stdout);
        return why == DLL_PROCESS_DETACH || want[0] == '1';
    }
    return TRUE;
}
#else
#define FAIL (sizeof(void *) == 8 ? "dllinit-fail-64.dll" : "dllinit-fail-32.dll")
#define DEP (sizeof(void *) == 8 ? "dllinit-dep-64.dll" : "dllinit-dep-32.dll")

static void loaded(void) {
    printf("  loaded: fail %d dep %d\n", GetModuleHandleA(FAIL) != NULL, GetModuleHandleA(DEP) != NULL);
}

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    for (int round = 0; round < 2; round++) {
        SetEnvironmentVariableA("DLLINIT_ANSWER", "0");
        printf("LoadLibrary fail (DllMain answers FALSE), round %d\n", round);
        SetLastError(0xdead);
        HMODULE h = LoadLibraryA(FAIL);
        printf("  got it %d error %lu\n", h != NULL, h ? 0 : GetLastError());
        loaded();
    }
    printf("LoadLibrary dep, then fail (FALSE): dep stays\n");
    HMODULE d = LoadLibraryA(DEP);
    SetLastError(0xdead);
    HMODULE h = LoadLibraryA(FAIL);
    printf("  got it %d error %lu\n", h != NULL, h ? 0 : GetLastError());
    loaded();
    printf("LoadLibrary fail (DllMain answers TRUE)\n");
    SetEnvironmentVariableA("DLLINIT_ANSWER", "1");
    h = LoadLibraryA(FAIL);
    printf("  got it %d\n", h != NULL);
    int (*f)(void) = h ? (int (*)(void))GetProcAddress(h, "fail_value") : NULL;
    printf("  fail_value %d\n", f ? f() : -1);
    printf("FreeLibrary fail %d\n", h ? FreeLibrary(h) : -1);
    loaded();
    printf("FreeLibrary dep %d\n", FreeLibrary(d));
    loaded();
    printf("end\n");
    return 0;
}
#endif
