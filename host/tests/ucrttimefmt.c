#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <time.h>

typedef struct TimeData {
    const char *narrow[43];
    int calendar;
    int references;
    const WCHAR *wide[43];
    const WCHAR *locale;
} TimeData;
typedef size_t (__cdecl *Format)(char *, size_t, const char *, const struct tm *, const TimeData *);

typedef size_t (__cdecl *FormatWide)(WCHAR *, size_t, const WCHAR *, const struct tm *, const TimeData *);

static int rejected_count;
static void __cdecl rejected(const WCHAR *expression, const WCHAR *function, const WCHAR *file,
                             unsigned line, uintptr_t reserved)
{
    (void)expression; (void)function; (void)file; (void)line; (void)reserved;
    ++rejected_count;
}

int main(void)
{
    HMODULE ucrt = LoadLibraryA("ucrtbase.dll");
    Format format = (Format)(uintptr_t)GetProcAddress(ucrt, "_Strftime");
    if (!format) return 1;

    TimeData locale = {0};
    locale.calendar = 1;
    locale.wide[2] = L"wk";
    locale.wide[9] = L"weekday";
    locale.wide[16] = L"mo";
    locale.wide[28] = L"month";
    locale.wide[38] = L"morn";
    locale.wide[39] = L"eve";
    locale.wide[40] = L"dd.M.yy";
    locale.wide[41] = L"ddddd MMMMM yyyyy gg";
    locale.wide[42] = L"hhh:Hmm mmm sss t/ttt/a/A";
    struct tm tm = { 7, 5, 13, 4, 2, 124, 2, 0, 0 };
    char buf[128];
    static const char *inputs[] = { "%a/%A/%b/%h/%B/%p", "%x", "%#x", "%X", "%r", "%c", "%#c", "%Y/%d", 0 };
    for (const char **it = inputs; *it; it++) {
        size_t len = format(buf, sizeof(buf), *it, &tm, &locale);
        printf("%s %zu [%s]\n", *it, len, buf);
    }
    locale.wide[40] = L"'literal Y M d' '' 'a''b' yyyy y yyy";
    size_t len = format(buf, sizeof(buf), "%x", &tm, &locale);
    printf("quoted %zu [%s]\n", len, buf);
    len = format(buf, 4, "%x", &tm, &locale);
    printf("short %zu [%s]\n", len, buf);
    FormatWide format_wide = (FormatWide)(uintptr_t)GetProcAddress(ucrt, "_Wcsftime");
    if (!format_wide) return 1;
    WCHAR wide[128];
    len = format_wide(wide, sizeof(wide) / sizeof(*wide), L"%a, %p", &tm, &locale);
    printf("wide %zu matches %d\n", len, wcscmp(wide, L"wk, eve") == 0);
    typedef size_t (__cdecl *DefaultFormat)(char *, size_t, const char *, const struct tm *);
    typedef void (__cdecl *InvalidHandler)(const WCHAR *, const WCHAR *, const WCHAR *, unsigned, uintptr_t);
    typedef InvalidHandler (__cdecl *Install)(InvalidHandler);
    DefaultFormat primary = (DefaultFormat)(uintptr_t)GetProcAddress(ucrt, "strftime");
    Install install = (Install)(uintptr_t)GetProcAddress(ucrt, "_set_invalid_parameter_handler");
    if (!primary || !install) return 1;
    static const char *builtins[] = { "%c", "%#c", "%#D", "%#F", "%#R", "%#T", "%#x", "%#X", "%#r", 0 };
    for (const char **it = builtins; *it; it++) {
        len = primary(buf, sizeof(buf), *it, &tm);
        printf("default %s %zu [%s]\n", *it, len, buf);
    }
    InvalidHandler old = install(rejected);
    len = primary(buf, sizeof(buf), "%Q", &tm);
    printf("invalid %zu [%s] handler %d\n", len, buf, rejected_count);
    install(old);
    return 0;
}
