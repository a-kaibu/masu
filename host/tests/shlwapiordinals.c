/* Fixed-size Shell arrays on caller memory, heap growth, and ordinal integer lookup. */
#include <windows.h>
#include <stdio.h>
#include <string.h>
typedef struct {
    DWORD count;
    unsigned char *buffer;
    DWORD capacity;
    BYTE growth, size, flags;
} Fdsa;
int main(void) {
    HMODULE dll = LoadLibraryA("shlwapi.dll");
    BOOL (WINAPI *init)(DWORD,DWORD,Fdsa*,void*,DWORD) = (void *)GetProcAddress(dll, MAKEINTRESOURCEA(208));
    BOOL (WINAPI *destroy)(Fdsa*) = (void *)GetProcAddress(dll, MAKEINTRESOURCEA(209));
    DWORD (WINAPI *insert)(Fdsa*,DWORD,const void*) = (void *)GetProcAddress(dll, MAKEINTRESOURCEA(210));
    BOOL (WINAPI *delete)(Fdsa*,DWORD) = (void *)GetProcAddress(dll, MAKEINTRESOURCEA(211));
    int (WINAPI *search)(const int*,const int*,int,int) = (void *)GetProcAddress(dll, MAKEINTRESOURCEA(198));
    if (!init || !destroy || !insert || !delete || !search) return 1;
    Fdsa a;
    unsigned char base[8];
    memset(&a, 0xcc, sizeof(a));
    memset(base, 0xcc, sizeof(base));
    BOOL ok = init(2, 2, &a, base, 4);
    printf("init %d %lu %lu %d %d %d %d %02x\n", ok, a.count, a.capacity, a.growth, a.size, a.flags, a.buffer == base, base[0]);
    DWORD index = insert(&a, 999, "ab");
    printf("append %lu %.2s\n", index, a.buffer);
    insert(&a, 0, "cd");
    insert(&a, 1, "ef");
    insert(&a, 9, "gh");
    index = insert(&a, 0, "ij");
    printf("grow %lu %lu %lu %d %d %.10s %02x\n", index, a.count, a.capacity, a.flags, a.buffer != base, a.buffer, a.buffer[10]);
    insert(&a, 9, "kl");
    insert(&a, 9, "mn");
    printf("regrow %lu %lu %d %.14s %02x\n", a.count, a.capacity, a.flags, a.buffer, a.buffer[14]);
    ok = delete(&a, 1);
    printf("delete %d %lu %.12s %02x %02x\n", ok, a.count, a.buffer, a.buffer[12], a.buffer[13]);
    printf("missing %d\n", delete(&a, 99));
    ok = destroy(&a);
    printf("destroy-owned %d\n", ok);
    memset(base, 0xcc, sizeof(base));
    ok = init(2, 0, &a, base, 4);
    printf("default %d %d %d %d\n", ok, a.growth, a.size, a.flags);
    printf("destroy-borrowed %d\n", destroy(&a));
    ok = init(2, 3, &a, NULL, 0);
    printf("empty %d %lu %lu %d\n", ok, a.count, a.capacity, a.flags);
    index = insert(&a, 0, "op");
    printf("first %lu %lu %lu %d %.2s\n", index, a.count, a.capacity, a.flags, a.buffer);
    destroy(&a);
    int keys[] = {3, -123, -123}, values[] = {4, 99, 80};
    printf("search %d %d %d %d %d\n", search(keys, values, 3, -123), search(keys, values, 1, 3), search(keys, values, 3, 5), search(keys, values, 0, 3), search(keys, values, -1, 3));
    return 0;
}
