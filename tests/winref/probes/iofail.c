/* What a read or a write says when it cannot move the bytes (review 6852be1f: sake's WriteFile
 * from a buffer that is not memory wrote nothing and answered TRUE with the count asked, and a
 * read error was the end of the file). ReadFile / WriteFile / _lread / _lwrite / _read / _write
 * with a buffer in the NULL pages and one past the user address space, a read at the end of the
 * file (TRUE, 0 bytes), a write of 0 bytes, and what the file holds afterwards. */
#include "../probe.h"
#include <io.h>
#include <fcntl.h>
#include <errno.h>

#define E(expr) do { SetLastError(0xdead); long long r_ = (long long)(expr); printf("%-60s = %lld err %lu\n", #expr, r_, GetLastError()); } while (0)

int main(void) {
    char path[MAX_PATH];
    GetTempPathA(MAX_PATH, path);
    lstrcatA(path, "winref-iofail.bin");
    HANDLE h = CreateFileA(path, GENERIC_READ | GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, 0, NULL);
    DWORD n = 0xdead;
    void *bad[] = { (void *)0x10, (void *)(ULONG_PTR)0xfffff000u };
    E(WriteFile(h, "abcdef", 6, &n, NULL));
    printf("  written %lu\n", n);
    for (int i = 0; i < 2; i++) {
        n = 0xdead;
        E(WriteFile(h, bad[i], 16, &n, NULL));
        printf("  written %lu\n", n);
    }
    n = 0xdead;
    E(WriteFile(h, bad[0], 0, &n, NULL));
    printf("  written %lu\n", n);
    printf("size %lu\n", GetFileSize(h, NULL));

    SetFilePointer(h, 0, NULL, FILE_BEGIN);
    for (int i = 0; i < 2; i++) {
        n = 0xdead;
        E(ReadFile(h, bad[i], 4, &n, NULL));
        printf("  read %lu\n", n);
    }
    char buf[16] = { 0 };
    n = 0xdead;
    E(ReadFile(h, buf, sizeof buf, &n, NULL));
    printf("  read %lu \"%.*s\"\n", n, (int)n, buf);
    n = 0xdead;
    E(ReadFile(h, buf, sizeof buf, &n, NULL));
    printf("  read at the end %lu\n", n);
    CloseHandle(h);

    // the 16-bit era calls and the C runtime's
    HFILE hf = _lopen(path, OF_READWRITE);
    E(_llseek(hf, 0, 2));
    E(_lwrite(hf, (LPCCH)bad[0], 8));
    E(_lwrite(hf, "gh", 2));
    E(_llseek(hf, 0, 0));
    E(_lread(hf, bad[0], 4));
    E(_lread(hf, buf, sizeof buf));
    E(_lread(hf, buf, sizeof buf));
    _lclose(hf);

    int fd = _open(path, _O_RDWR | _O_BINARY);
    _lseek(fd, 0, SEEK_END);
    errno = 0;
    E(_write(fd, bad[0], 8));
    printf("  errno %d\n", errno);
    E(_write(fd, "ij", 2));
    _lseek(fd, 0, SEEK_SET);
    errno = 0;
    E(_read(fd, bad[0], 4));
    printf("  errno %d\n", errno);
    E(_read(fd, buf, sizeof buf));
    printf("  \"%.10s\"\n", buf);
    errno = 0;
    E(_read(fd, buf, sizeof buf));
    printf("  at the end errno %d\n", errno);
    _close(fd);
    DeleteFileA(path);
    return 0;
}
