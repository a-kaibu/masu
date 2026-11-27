#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef void (__cdecl *Invalid)(const wchar_t *, const wchar_t *, const wchar_t *, unsigned, uintptr_t);
typedef void (__cdecl *NoInfo)(void);
typedef Invalid (__cdecl *Install)(Invalid);
typedef int (__cdecl *Copy)(char *, size_t, const char *);

static void __cdecl handler(const wchar_t *expr, const wchar_t *fn, const wchar_t *file,
                            unsigned line, uintptr_t reserved)
{
    printf("handler %u %d %d %d %llx\n", line, expr != NULL, fn != NULL, file != NULL,
           (unsigned long long)reserved);
    fflush(stdout);
}

int main(int argc, char **argv)
{
    HMODULE crt = LoadLibraryA("ucrtbase.dll");
    if (argc == 2) {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
        const char *mode = argv[1];
        if (!strcmp(mode, "legacy")) crt = LoadLibraryA("msvcrt.dll");
        NoInfo noinfo = (NoInfo)(uintptr_t)GetProcAddress(crt, "_invalid_parameter_noinfo");
        NoInfo noreturn = (NoInfo)(uintptr_t)GetProcAddress(crt, "_invalid_parameter_noinfo_noreturn");
        Install install = (Install)(uintptr_t)GetProcAddress(crt, "_set_invalid_parameter_handler");
        Copy copy = (Copy)(uintptr_t)GetProcAddress(crt, "strcpy_s");
        if (!strcmp(mode, "noinfo")) {
            if (!noinfo) return 92;
            noinfo();
        } else if (!strcmp(mode, "noreturn")) {
            if (!noreturn) return 93;
            noreturn();
        } else if (!strcmp(mode, "handler") || !strcmp(mode, "noreturn-handler")) {
            if (!install || !noinfo || !noreturn || !copy) return 94;
            install(handler);
            if (!strcmp(mode, "handler")) {
                noinfo();
                return copy(NULL, 0, "x");
            } else noreturn();
        } else {
            if (!copy) return 95;
            return copy(NULL, 0, "x");
        }
        return 42;
    }

    char exe[MAX_PATH];
    GetModuleFileNameA(NULL, exe, sizeof(exe));
    const char *modes[] = {"noinfo", "noreturn", "handler", "noreturn-handler", "legacy", "secure"};
    for (unsigned i = 0; i < sizeof(modes) / sizeof(modes[0]); i++) {
        char command[MAX_PATH + 64];
        snprintf(command, sizeof(command), "\"%s\" %s", exe, modes[i]);
        STARTUPINFOA si = {0};
        PROCESS_INFORMATION pi;
        DWORD status = 0;
        si.cb = sizeof(si);
        if (!CreateProcessA(NULL, command, NULL, NULL, TRUE, 0, NULL, NULL, &si, &pi)) return 2;
        if (WaitForSingleObject(pi.hProcess, 10000) != WAIT_OBJECT_0) return 3;
        GetExitCodeProcess(pi.hProcess, &status);
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        printf("%s %08lx\n", modes[i], (unsigned long)status);
        fflush(stdout);
    }
    return 0;
}
