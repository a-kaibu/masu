#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <math.h>

typedef struct MathException {
    int type;
    const char *name;
    double x, y, result;
} MathException;
typedef void (__cdecl *Install)(int (__cdecl *)(MathException *));
typedef int *(__cdecl *ErrnoPtr)(void);
typedef double (__cdecl *Unary)(double);
typedef double (__cdecl *Binary)(double, double);
typedef float (__cdecl *UnaryF)(float);
typedef int (__cdecl *MaskFn)(int);
#define LOAD(type, symbol) (type)(uintptr_t)GetProcAddress(lib, symbol)

static int handled, type, failed;
static double x_arg, y_arg;
static char name[32];

static int __cdecl report(MathException *e)
{
    type = e->type;
    x_arg = e->x;
    y_arg = strcmp(e->name, "pow") == 0 ? e->y : 0;
    snprintf(name, sizeof(name), "%s", e->name);
    e->result = 42.5;
    return handled;
}

#define CHECK(label, predicate) do {                          \
    int ok = !!(predicate);                                   \
    printf("%s: %s\n", label, ok ? "ok" : "FAIL");             \
    failed |= !ok;                                            \
} while (0)

int main(void)
{
    HMODULE lib = LoadLibraryA("ucrtbase.dll");
    Install install = LOAD(Install, "__setusermatherr");
    ErrnoPtr where = LOAD(ErrnoPtr, "_errno");
    Unary ex = LOAD(Unary, "exp"), sq = LOAD(Unary, "sqrt"), ln = LOAD(Unary, "log");
    Unary rint_fn = LOAD(Unary, "rint");
    Binary power = LOAD(Binary, "pow");
    MaskFn set_round = LOAD(MaskFn, "fesetround");
    MaskFn test_flags = LOAD(MaskFn, "fetestexcept"), clear = LOAD(MaskFn, "feclearexcept");
    if (!install || !where || !ex || !sq || !ln || !power || !rint_fn ||
        !set_round || !test_flags || !clear) return 1;

    int *error = where();
    install(report);
    const int x64 = sizeof(void *) == 8;
    *error = -1;
    double r = sq(-5);
    CHECK("sqrt domain", r == 42.5 && *error == 33 && type == 1 &&
          strcmp(name, "sqrt") == 0 && x_arg == -5);

    handled = 1;
    *error = -1;
    r = ln(0);
    CHECK("log singularity", (x64 ? isinf(r) && r < 0 : r == 42.5) &&
          *error == -1 && type == 2 && strcmp(name, "log") == 0 && x_arg == 0);

    r = ex(900);
    CHECK("exp overflow", (x64 ? isinf(r) && r > 0 : r == 42.5) &&
          *error == -1 && type == 3 && strcmp(name, "exp") == 0 && x_arg == 900);

    handled = 0;
    r = ex(-746);
    CHECK("exp underflow", r == (x64 ? 0.0 : 42.5) && *error == -1 &&
          type == 4 && strcmp(name, "exp") == 0 && x_arg == -746);

    type = 0;
    r = power(0, -2);
    CHECK("pow singularity", *error == 34 && (x64 ? isinf(r) && r > 0 && type == 0 :
          r == 42.5 && type == 2 && strcmp(name, "pow") == 0 && x_arg == 0 && y_arg == -2));

    if (x64) {
        UnaryF exf = LOAD(UnaryF, "expf");
        if (!exf) return 1;
        type = 0;
        *error = -1;
        float rf = exf(90);
        if (!(isinf(rf) && rf > 0 && *error == 34 &&
              type == 3 && strcmp(name, "expf") == 0 && x_arg == 90)) {
            puts("expf overflow on x64: FAIL");
            failed = 1;
        }
    }

    install(NULL);
    *error = -1;
    type = 0;
    r = sq(-1);
    CHECK("handler removed", isnan(r) && *error == 33 && type == 0);
    const double positive[] = {2, 2, 3, 2}, negative[] = {-2, -3, -2, -2};
    int rounding = 1;
    for (int mode = 0; mode < 4; mode++) {
        clear(31);
        rounding &= set_round(mode << 8) == 0;
        rounding &= rint_fn(2.5) == positive[mode] && rint_fn(-2.5) == negative[mode];
        rounding &= test_flags(31) == 1;
    }
    CHECK("rint rounding and inexact flag", rounding);
    set_round(0);
    return failed;
}
