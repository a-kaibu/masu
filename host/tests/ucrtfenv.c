#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <xmmintrin.h>

typedef struct Env { uint32_t control, status; } Env;
typedef int (__cdecl *EnvFn)(Env *);
typedef int (__cdecl *MaskFn)(int);
typedef int (__cdecl *FlagFn)(uint32_t *, int);
typedef int (__cdecl *GetRound)(void);

#define LOAD(type, symbol) (type)(uintptr_t)GetProcAddress(lib, symbol)

int main(void)
{
    HMODULE lib = LoadLibraryA("ucrtbase.dll");
    EnvFn read_env = LOAD(EnvFn, "fegetenv"), put_env = LOAD(EnvFn, "fesetenv");
    EnvFn hold = LOAD(EnvFn, "feholdexcept");
    MaskFn set_round = LOAD(MaskFn, "fesetround");
    GetRound get_round = LOAD(GetRound, "fegetround");
    FlagFn get_flag = LOAD(FlagFn, "fegetexceptflag"), set_flag = LOAD(FlagFn, "fesetexceptflag");
    MaskFn test = LOAD(MaskFn, "fetestexcept"), clear = LOAD(MaskFn, "feclearexcept");
    if (!read_env || !put_env || !hold || !set_round || !get_round || !get_flag || !set_flag || !test || !clear) return 1;
    Env initial = {0}, changed = {0};
#if defined(__i386__)
    const uint16_t cw = 0x037f;
    __asm__ volatile ("fninit; fldcw %0" : : "m"(cw));
#endif
    _mm_setcsr(0x1f80);
    read_env(&initial);
    const unsigned unit_bits =
#if defined(__i386__)
        0x00ff0000;
#else
        0;
#endif
    printf("default %08x %08x\n", initial.control & ~unit_bits, initial.status);
    for (unsigned mode = 0; mode <= 0x400; mode += 0x100) {
        int result = set_round(mode);
        read_env(&changed);
        printf("round %03x result %d now %03x ctrl %08x mx %04x\n", mode, result, get_round(), changed.control & ~unit_bits, _mm_getcsr());
    }
    set_round(0);
    for (unsigned bits = 0; bits <= 3; bits++) {
        const unsigned csr = 0x1f80 | (bits & 1 ? 0x40 : 0) | (bits & 2 ? 0x8000 : 0);
        _mm_setcsr(csr);
        read_env(&changed);
        int saved = put_env(&changed);
        printf("denorm %x ctrl %08x save %d mx %04x\n", bits, changed.control & ~unit_bits, saved, _mm_getcsr());
    }
    Env invalid = { initial.control, 1 };
    printf("invalid %d\n", put_env(&invalid));
    _mm_setcsr(0x1f04);
    Env held;
    int held_ok = hold(&held);
#if defined(__i386__)
    const int expected_hold = 0;
    const unsigned expected_control = 0x1f00;
#else
    const int expected_hold = 1;
    const unsigned expected_control = 0x1f04;
#endif
    printf("hold matches %d status %08x\n",
           held_ok == expected_hold && _mm_getcsr() == expected_control, held.status);
    put_env(&initial);
    printf("restored %03x\n", get_round());
    _mm_setcsr(0x1f80 | 0x06);
    read_env(&changed);
    uint32_t flags = 0;
    get_flag(&flags, 31);
    printf("sse status %08x except %08x test %02x\n", changed.status, flags, test(31));
    clear(8);
    read_env(&changed);
    printf("sse clear %08x\n", changed.status);
    flags = 0x10000010;
    int result = set_flag(&flags, 31);
    read_env(&changed);
    printf("set flags %d stat %08x test %02x\n", result, changed.status, test(31));
    put_env(&initial);
    printf("end %02x\n", test(31));
    return 0;
}
