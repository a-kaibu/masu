#include <windows.h>
#include <stdio.h>

int main(void)
{
    DWORD probes[] = {0, 31, 32, 33, 63, 64, 65};
    for (unsigned i = 0; i < sizeof(probes) / sizeof(probes[0]); ++i) {
        SetLastError(0xdeadbeef);
        DWORD previous = SetThreadIdealProcessor(GetCurrentThread(), probes[i]);
        printf("ideal %lu success %d error %lu\n", probes[i], previous != (DWORD)-1,
               previous == (DWORD)-1 ? GetLastError() : 0);
    }
    return 0;
}
