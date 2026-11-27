/* The Win16 INI reads place the default, buffer and size in separate arguments. */
#include <windows.h>
#include <stdio.h>
#include <string.h>

int PASCAL WinMain(HINSTANCE hi, HINSTANCE hp, LPSTR cmd, int sh) {
    const char far *ini = "C:\\profile16.ini";
    char b[32];
    UINT n;
    printf("write %d\n", WritePrivateProfileString("probe", "key", "sample", ini) != 0);
    n = GetPrivateProfileString("probe", "key", "default", b, sizeof(b), ini);
    printf("private %u %s\n", n, b);
    memset(b, 'x', sizeof(b));
    n = GetPrivateProfileString("probe", "missing", "default", b, 5, ini);
    printf("private default %u %s tail %c\n", n, b, b[5]);
    n = GetProfileString("unknown16", "missing", "profile", b, sizeof(b));
    printf("profile default %u %s\n", n, b);
    return 0;
}
