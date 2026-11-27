/* A code segment can be read through a data alias and copied to another selector. */
#include <windows.h>
#include <stdio.h>
#include <i86.h>

WORD FAR PASCAL AllocCStoDSAlias(WORD);

int FAR PASCAL __export Answer(void) {
    return 42;
}

int PASCAL WinMain(HINSTANCE hi, HINSTANCE hp, LPSTR cmd, int show) {
    typedef int (FAR PASCAL *Func)(void);
    void FAR *entry = (void FAR *)Answer;
    WORD src = FP_SEG(entry);
    WORD off = FP_OFF(entry);
    WORD alias = AllocCStoDSAlias(src);
    WORD data = AllocSelector(0);
    WORD rebound;
    WORD code, released;
    Func f;
    if (!alias || !data) {
        puts("selector allocation failed");
        return 1;
    }
    printf("code read alias %d\n", alias != src && *(BYTE FAR *)MK_FP(alias, off) == *(BYTE FAR *)entry);
    rebound = PrestoChangoSelector(src, data);
    printf("copy into supplied selector %d\n",
        rebound == data && *(BYTE FAR *)MK_FP(data, off) == *(BYTE FAR *)entry);
    code = AllocDStoCSAlias(data);
    if (!code) return 2;
    released = FreeSelector(alias) | FreeSelector(data);
    f = (Func)MK_FP(code, off);
    printf("call through alias %d, release %d\n", f(), released == 0);
    printf("original stays callable %d\n", FreeSelector(code) == 0 && Answer() == 42);
    return 0;
}
