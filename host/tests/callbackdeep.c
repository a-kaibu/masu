/* Reentrant SendMessage uses 16 KiB of guest stack per callback. The bounded case
   returns through 80 frames. The unbounded case reaches STATUS_STACK_OVERFLOW. */
#include <windows.h>
#include <malloc.h>
#include <stdio.h>
#include <string.h>

static int depth, limit;

static LRESULT CALLBACK proc(HWND h, UINT m, WPARAM w, LPARAM l) {
    if (m != WM_USER) return DefWindowProcA(h, m, w, l);
    volatile char *p = _alloca(16384);
    memset((char *)p, depth, 16384);
    depth++;
    LRESULT result = depth == limit ? 0 : SendMessageA(h, WM_USER, 0, 0);
    return result + p[100];
}

static DWORD run_child(const char *mode) {
    char exe[MAX_PATH], cmd[MAX_PATH + 20];
    GetModuleFileNameA(NULL, exe, MAX_PATH);
    wsprintfA(cmd, "\"%s\" %s", exe, mode);

    /* Keep a child's diagnostic output out of the parent's result. */
    SECURITY_ATTRIBUTES sa = {sizeof(sa), NULL, TRUE};
    HANDLE nul = CreateFileA("NUL", GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE,
            &sa, OPEN_EXISTING, 0, NULL);
    STARTUPINFOA si = {sizeof(si)};
    PROCESS_INFORMATION pi;
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdInput = si.hStdOutput = si.hStdError = nul;
    BOOL started = CreateProcessA(NULL, cmd, NULL, NULL, TRUE, 0, NULL, NULL, &si, &pi);
    CloseHandle(nul);
    if (!started) return 1;

    DWORD code = 1;
    if (WaitForSingleObject(pi.hProcess, 60000) == WAIT_OBJECT_0) {
        GetExitCodeProcess(pi.hProcess, &code);
    } else {
        TerminateProcess(pi.hProcess, 1);
    }
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    return code;
}

int main(int argc, char **argv) {
    if (argc > 1) {
        limit = !strcmp(argv[1], "bounded") ? 80 : 0;
        WNDCLASSA wc = {0};
        wc.lpfnWndProc = proc;
        wc.lpszClassName = "deep";
        if (!RegisterClassA(&wc)) return 1;
        HWND h = CreateWindowA("deep", "", 0, 0, 0, 10, 10, NULL, NULL, NULL, NULL);
        if (!h) return 1;
        LRESULT sum = SendMessageA(h, WM_USER, 0, 0);
        return limit == 80 && depth == 80 && sum == 79 * 80 / 2 ? 0 : 1;
    }

    DWORD bounded = run_child("bounded");
    DWORD overflow = run_child("overflow");
    printf("80 nested callbacks exit %08lx\n", bounded);
    printf("child exit %08lx\n", overflow);
    return bounded != 0 || overflow != 0xc00000fd;
}
