/* Which of two DLLs of one name a load picks, as the search rules say: the application's
 * folder, AddDllDirectory's, SetDllDirectory's, LoadLibraryEx's LOAD_LIBRARY_SEARCH_* flags,
 * SetDefaultDllDirectories' (for LoadLibrary and LoadLibraryEx with no flags), and where a DLL
 * loaded by path finds its own imports (LOAD_WITH_ALTERED_SEARCH_PATH,
 * LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, neither). The same source builds dllsearch-a-NN.dll (which()
 * is 1), dllsearch-b-NN.dll (2), dllsearch-dep-NN.dll and dllsearch-lib-NN.dll importing it;
 * the probe copies them under new names into its folder and a "sub" folder beside it, one name
 * per case (a module once loaded answers its name). sake's loader searched an order of its own
 * and took none of the rules (review b5dacf4 2.1).
 * dlls: lib dep a b */
#include "../probe.h"

#ifdef PROBE_A
__declspec(dllexport) int which(void) { return 1; }
#elif defined PROBE_B
__declspec(dllexport) int which(void) { return 2; }
#elif defined PROBE_DEP
__declspec(dllexport) int dep_value(void) { return 7; }
#elif defined PROBE_LIB
__declspec(dllimport) int dep_value(void);
__declspec(dllexport) int which(void) { return 10 + dep_value(); }
#else
#define NN (sizeof(void *) == 8 ? "64" : "32")

static char app[MAX_PATH], sub[MAX_PATH];

static void place(const char *dll, const char *dir, const char *as) {
    char from[MAX_PATH], to[MAX_PATH];
    snprintf(from, sizeof from, "%s\\dllsearch-%s-%s.dll", app, dll, NN);
    snprintf(to, sizeof to, "%s\\%s", dir, as);
    if (!CopyFileA(from, to, FALSE)) printf("copy %s to %s failed %lu\n", dll, as, GetLastError());
}

static void report(const char *what, HMODULE h) {
    if (!h) {
        printf("%s: none err %lu\n", what, GetLastError());
        return;
    }
    int (*f)(void) = (int (*)(void))GetProcAddress(h, "which");
    printf("%s: which %d\n", what, f ? f() : -1);
}

static HMODULE ex(const char *name, DWORD flags) {
    WCHAR w[MAX_PATH];
    MultiByteToWideChar(CP_ACP, 0, name, -1, w, MAX_PATH);
    SetLastError(0xdead);
    return LoadLibraryExW(w, NULL, flags);
}

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    GetModuleFileNameA(NULL, app, MAX_PATH);
    *strrchr(app, '\\') = 0;
    snprintf(sub, sizeof sub, "%s\\dllsearch-sub", app);
    CreateDirectoryA(sub, NULL);
    SetCurrentDirectoryA(app);
    HMODULE k32 = GetModuleHandleA("kernel32.dll");
    void *(WINAPI *add)(PCWSTR) = (void *(WINAPI *)(PCWSTR))GetProcAddress(k32, "AddDllDirectory");
    BOOL(WINAPI *defaults)(DWORD) = (BOOL(WINAPI *)(DWORD))GetProcAddress(k32, "SetDefaultDllDirectories");
    if (!add || !defaults) {
        printf("skip no AddDllDirectory\n");
        return 0;
    }

    /* A in the application's folder, B in sub: the default search takes A */
    place("a", app, "p1.dll"), place("b", sub, "p1.dll");
    SetLastError(0xdead);
    report("LoadLibrary, app and sub", LoadLibraryA("p1.dll"));
    /* only in sub: not found without a rule naming it */
    place("b", sub, "p2.dll");
    SetLastError(0xdead);
    report("LoadLibrary, sub only", LoadLibraryA("p2.dll"));
    /* SetDllDirectory(sub): searched after the application's folder */
    SetDllDirectoryA(sub);
    place("b", sub, "p3.dll");
    SetLastError(0xdead);
    report("SetDllDirectory sub, sub only", LoadLibraryA("p3.dll"));
    place("a", app, "p4.dll"), place("b", sub, "p4.dll");
    SetLastError(0xdead);
    report("SetDllDirectory sub, app and sub", LoadLibraryA("p4.dll"));
    SetDllDirectoryA(NULL);

    WCHAR wsub[MAX_PATH];
    MultiByteToWideChar(CP_ACP, 0, sub, -1, wsub, MAX_PATH);
    void *cookie = add(wsub);
    printf("AddDllDirectory %d\n", cookie != NULL);
    /* AddDllDirectory alone changes nothing for a plain load */
    place("a", app, "p5.dll"), place("b", sub, "p5.dll");
    report("AddDllDirectory sub, LoadLibrary", LoadLibraryA("p5.dll"));
    /* the flags: user folders only, the application's only, both (application first), system32 */
    place("a", app, "p6.dll"), place("b", sub, "p6.dll");
    report("LOAD_LIBRARY_SEARCH_USER_DIRS", ex("p6.dll", LOAD_LIBRARY_SEARCH_USER_DIRS));
    place("a", app, "p7.dll"), place("b", sub, "p7.dll");
    report("LOAD_LIBRARY_SEARCH_APPLICATION_DIR", ex("p7.dll", LOAD_LIBRARY_SEARCH_APPLICATION_DIR));
    place("a", app, "p8.dll"), place("b", sub, "p8.dll");
    report("LOAD_LIBRARY_SEARCH_DEFAULT_DIRS", ex("p8.dll", LOAD_LIBRARY_SEARCH_DEFAULT_DIRS));
    place("a", app, "p9.dll");
    report("LOAD_LIBRARY_SEARCH_SYSTEM32, app only", ex("p9.dll", LOAD_LIBRARY_SEARCH_SYSTEM32));
    report("LOAD_LIBRARY_SEARCH_USER_DIRS, app only", ex("p9.dll", LOAD_LIBRARY_SEARCH_USER_DIRS));
    /* a relative name with DLL_LOAD_DIR, the altered search with another flag: refused */
    report("DLL_LOAD_DIR, relative name", ex("p9.dll", LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR));
    report("altered and USER_DIRS", ex("p9.dll", LOAD_WITH_ALTERED_SEARCH_PATH | LOAD_LIBRARY_SEARCH_USER_DIRS));

    /* a DLL loaded by path whose import is beside it only */
    char dep[MAX_PATH], depsub[MAX_PATH], lib[MAX_PATH], lib2[MAX_PATH], lib3[MAX_PATH];
    snprintf(dep, sizeof dep, "%s\\dllsearch-dep-%s.dll", app, NN);
    snprintf(depsub, sizeof depsub, "%s\\dllsearch-dep-%s.dll", sub, NN);
    if (!MoveFileExA(dep, depsub, MOVEFILE_REPLACE_EXISTING)) printf("move dep failed %lu\n", GetLastError());
    place("lib", sub, "l1.dll"), place("lib", sub, "l2.dll"), place("lib", sub, "l3.dll");
    snprintf(lib, sizeof lib, "%s\\l1.dll", sub);
    snprintf(lib2, sizeof lib2, "%s\\l2.dll", sub);
    snprintf(lib3, sizeof lib3, "%s\\l3.dll", sub);
    SetLastError(0xdead);
    report("by path, import beside it, no flag", LoadLibraryA(lib));
    HMODULE h = ex(lib2, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32);
    report("by path, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR", h);
    if (h) FreeLibrary(h);
    h = ex(lib3, LOAD_WITH_ALTERED_SEARCH_PATH);
    report("by path, LOAD_WITH_ALTERED_SEARCH_PATH", h);
    if (h) FreeLibrary(h);
    if (!MoveFileExA(depsub, dep, MOVEFILE_REPLACE_EXISTING)) printf("move dep back failed %lu\n", GetLastError());

    /* SetDefaultDllDirectories: LoadLibrary and LoadLibraryEx without flags take it */
    printf("SetDefaultDllDirectories(USER_DIRS) %d\n", defaults(LOAD_LIBRARY_SEARCH_USER_DIRS));
    place("a", app, "q1.dll"), place("b", sub, "q1.dll");
    report("defaults USER_DIRS, LoadLibraryEx 0", ex("q1.dll", 0));
    place("a", app, "q2.dll"), place("b", sub, "q2.dll");
    SetLastError(0xdead);
    report("defaults USER_DIRS, LoadLibrary", LoadLibraryA("q2.dll"));
    place("a", app, "q3.dll"), place("b", sub, "q3.dll");
    report("defaults USER_DIRS, flag APPLICATION_DIR", ex("q3.dll", LOAD_LIBRARY_SEARCH_APPLICATION_DIR));
    printf("SetDefaultDllDirectories(SYSTEM32) %d\n", defaults(LOAD_LIBRARY_SEARCH_SYSTEM32));
    place("a", app, "q4.dll");
    report("defaults SYSTEM32, app only", ex("q4.dll", 0));
    printf("SetDefaultDllDirectories(DEFAULT_DIRS) %d\n", defaults(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS));
    place("a", app, "q5.dll"), place("b", sub, "q5.dll");
    report("defaults DEFAULT_DIRS, app and sub", ex("q5.dll", 0));
    place("b", sub, "q6.dll");
    report("defaults DEFAULT_DIRS, sub only", ex("q6.dll", 0));
    return 0;
}
#endif
