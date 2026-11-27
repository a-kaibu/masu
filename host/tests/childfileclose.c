/* libs: -municode */
#include <windows.h>
#include <stdio.h>
#include <wchar.h>

static void names(DWORD pid, WCHAR *file, WCHAR *ready, WCHAR *finish)
{
    swprintf(file, 96, L"masu-vfs-child-%lu.tmp", (unsigned long)pid);
    swprintf(ready, 96, L"Local\\MasuVfsReady-%lu", (unsigned long)pid);
    swprintf(finish, 96, L"Local\\MasuVfsFinish-%lu", (unsigned long)pid);
}

static int child(DWORD parent)
{
    WCHAR file[96], ready_name[96], finish_name[96];
    names(parent, file, ready_name, finish_name);
    HANDLE ready = OpenEventW(EVENT_MODIFY_STATE, FALSE, ready_name);
    HANDLE finish = OpenEventW(SYNCHRONIZE, FALSE, finish_name);
    HANDLE held = CreateFileW(file, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_DELETE, NULL, OPEN_EXISTING, 0, NULL);
    if (!ready || !finish || held == INVALID_HANDLE_VALUE || !SetEvent(ready)) return 2;
    if (WaitForSingleObject(finish, 10000) != WAIT_OBJECT_0) return 3;
    char byte = 0;
    DWORD bytes = 0;
    if (!ReadFile(held, &byte, 1, &bytes, NULL) || bytes != 1 || byte != 'x') return 4;
    /* ExitProcess has to close held, ready and finish. */
    ExitProcess(0);
}

int wmain(int argc, WCHAR **argv)
{
    if (argc == 3 && !wcscmp(argv[1], L"child")) return child(wcstoul(argv[2], NULL, 10));
    DWORD pid = GetCurrentProcessId();
    WCHAR file[96], ready_name[96], finish_name[96], exe[MAX_PATH], cmd[MAX_PATH + 48];
    names(pid, file, ready_name, finish_name);
    HANDLE seed = CreateFileW(file, GENERIC_WRITE, FILE_SHARE_READ, NULL, CREATE_NEW, 0, NULL);
    if (seed == INVALID_HANDLE_VALUE) return 4;
    DWORD written = 0;
    BOOL seeded = WriteFile(seed, "x", 1, &written, NULL);
    CloseHandle(seed);
    if (!seeded || written != 1) { DeleteFileW(file); return 4; }
    HANDLE ready = CreateEventW(NULL, TRUE, FALSE, ready_name);
    HANDLE finish = CreateEventW(NULL, TRUE, FALSE, finish_name);
    DWORD len = GetModuleFileNameW(NULL, exe, MAX_PATH);
    if (!ready || !finish || !len || len == MAX_PATH) return 5;
    swprintf(cmd, sizeof(cmd) / sizeof(*cmd), L"\"%ls\" child %lu", exe, (unsigned long)pid);
    STARTUPINFOW startup = {0};
    startup.cb = sizeof(startup);
    PROCESS_INFORMATION proc;
    if (!CreateProcessW(exe, cmd, NULL, NULL, FALSE, 0, NULL, NULL, &startup, &proc)) return 6;
    CloseHandle(proc.hThread);
    DWORD opened = WaitForSingleObject(ready, 10000);
    BOOL deleted = DeleteFileW(file);
    SetEvent(finish);
    DWORD ended = WaitForSingleObject(proc.hProcess, 10000);
    DWORD exit_code = 999;
    GetExitCodeProcess(proc.hProcess, &exit_code);
    if (ended != WAIT_OBJECT_0) TerminateProcess(proc.hProcess, 7);
    CloseHandle(proc.hProcess);
    CloseHandle(finish);
    CloseHandle(ready);

    printf("child held file %d deleted while open %d\n", opened == WAIT_OBJECT_0, !!deleted);
    printf("child exited %d code %lu\n", ended == WAIT_OBJECT_0, (unsigned long)exit_code);
    printf("name gone after exit %d\n", GetFileAttributesW(file) == INVALID_FILE_ATTRIBUTES);
    HANDLE replacement = CreateFileW(file, GENERIC_WRITE, 0, NULL, CREATE_NEW, 0, NULL);
    printf("recreated %d\n", replacement != INVALID_HANDLE_VALUE);
    if (replacement != INVALID_HANDLE_VALUE) CloseHandle(replacement);
    DeleteFileW(file);
    return 0;
}
