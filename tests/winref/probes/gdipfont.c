/* GDI+ family metrics, font units and LOGFONT round-trips using the same synthetic
 * TrueType face on Windows and sake. Generic and installed collections also get
 * probed with checks independent of their localized names and typefaces.
 * libs: -lgdiplus -lole32 -lgdi32 */
#include "../probe.h"

struct found { int calls, truetype, em; };
static int CALLBACK enum_tt(const LOGFONTW *lf, const TEXTMETRICW *tm, DWORD type, LPARAM lp) {
    struct found *f = (struct found *)lp;
    f->calls++;
    if (type & TRUETYPE_FONTTYPE) {
        f->truetype = 1;
        f->em = ((const NEWTEXTMETRICW *)tm)->ntmSizeEM;
    }
    return 0;
}

typedef int GpStatus;
typedef void GpFontFamily, GpFont, GpGraphics, GpFontCollection;
typedef struct { UINT32 GdiplusVersion; void *DebugEventCallback; BOOL SuppressBackgroundThread, SuppressExternalCodecs; } StartupInput;
GpStatus WINAPI GdiplusStartup(ULONG_PTR *, const StartupInput *, void *);
GpStatus WINAPI GdipCreateFontFamilyFromName(const WCHAR *, GpFontCollection *, GpFontFamily **);
GpStatus WINAPI GdipCloneFontFamily(GpFontFamily *, GpFontFamily **);
GpStatus WINAPI GdipDeleteFontFamily(GpFontFamily *);
GpStatus WINAPI GdipGetFamilyName(GpFontFamily *, WCHAR *, LANGID);
GpStatus WINAPI GdipGetEmHeight(GpFontFamily *, INT, UINT16 *);
GpStatus WINAPI GdipGetCellAscent(GpFontFamily *, INT, UINT16 *);
GpStatus WINAPI GdipGetCellDescent(GpFontFamily *, INT, UINT16 *);
GpStatus WINAPI GdipGetLineSpacing(GpFontFamily *, INT, UINT16 *);
GpStatus WINAPI GdipGetGenericFontFamilySansSerif(GpFontFamily **);
GpStatus WINAPI GdipGetGenericFontFamilySerif(GpFontFamily **);
GpStatus WINAPI GdipGetGenericFontFamilyMonospace(GpFontFamily **);
GpStatus WINAPI GdipNewInstalledFontCollection(GpFontCollection **);
GpStatus WINAPI GdipGetFontCollectionFamilyCount(GpFontCollection *, INT *);
GpStatus WINAPI GdipGetFontCollectionFamilyList(GpFontCollection *, INT, GpFontFamily **, INT *);
GpStatus WINAPI GdipCreateFont(GpFontFamily *, float, INT, INT, GpFont **);
GpStatus WINAPI GdipCreateFontFromLogfontW(HDC, const LOGFONTW *, GpFont **);
GpStatus WINAPI GdipDeleteFont(GpFont *);
GpStatus WINAPI GdipCloneFont(GpFont *, GpFont **);
GpStatus WINAPI GdipGetFontSize(GpFont *, float *);
GpStatus WINAPI GdipGetFontUnit(GpFont *, INT *);
GpStatus WINAPI GdipGetFontStyle(GpFont *, INT *);
GpStatus WINAPI GdipGetFamily(GpFont *, GpFontFamily **);
GpStatus WINAPI GdipGetLogFontW(GpFont *, GpGraphics *, LOGFONTW *);
GpStatus WINAPI GdipGetFontHeight(GpFont *, GpGraphics *, float *);
GpStatus WINAPI GdipGetFontHeightGivenDPI(GpFont *, float, float *);
GpStatus WINAPI GdipCreateFromHDC(HDC, GpGraphics **);
GpStatus WINAPI GdipDeleteGraphics(GpGraphics *);

GpStatus WINAPI GdipNewPrivateFontCollection(GpFontCollection **);
GpStatus WINAPI GdipDeletePrivateFontCollection(GpFontCollection **);
GpStatus WINAPI GdipPrivateAddMemoryFont(GpFontCollection *, const void *, INT);

/* A two-glyph TrueType face generated from a polygon and fixed design metrics.
 * UPM 1024, ascent 800, descent 200, line gap 24, one regular style. */
static const BYTE test_font[] = {
    0x00, 0x01, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x80, 0x00, 0x03, 0x00, 0x20, 0x4f, 0x53, 0x2f, 0x32,
    0x45, 0x39, 0x44, 0x5d, 0x00, 0x00, 0x01, 0x28, 0x00, 0x00, 0x00, 0x60, 0x63, 0x6d, 0x61, 0x70,
    0x00, 0x0c, 0x00, 0x94, 0x00, 0x00, 0x01, 0x90, 0x00, 0x00, 0x00, 0x34, 0x67, 0x6c, 0x79, 0x66,
    0x48, 0xbb, 0xc4, 0xca, 0x00, 0x00, 0x01, 0xcc, 0x00, 0x00, 0x00, 0x24, 0x68, 0x65, 0x61, 0x64,
    0x15, 0xf9, 0x66, 0xb4, 0x00, 0x00, 0x00, 0xac, 0x00, 0x00, 0x00, 0x36, 0x68, 0x68, 0x65, 0x61,
    0x05, 0xba, 0x01, 0xec, 0x00, 0x00, 0x00, 0xe4, 0x00, 0x00, 0x00, 0x24, 0x68, 0x6d, 0x74, 0x78,
    0x02, 0xb2, 0x00, 0x00, 0x00, 0x00, 0x01, 0x88, 0x00, 0x00, 0x00, 0x06, 0x6c, 0x6f, 0x63, 0x61,
    0x00, 0x12, 0x00, 0x00, 0x00, 0x00, 0x01, 0xc4, 0x00, 0x00, 0x00, 0x06, 0x6d, 0x61, 0x78, 0x70,
    0x00, 0x04, 0x00, 0x08, 0x00, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x20, 0x6e, 0x61, 0x6d, 0x65,
    0x81, 0x57, 0xb8, 0x2e, 0x00, 0x00, 0x01, 0xf0, 0x00, 0x00, 0x01, 0x2c, 0x70, 0x6f, 0x73, 0x74,
    0x00, 0x28, 0x00, 0x00, 0x00, 0x00, 0x03, 0x1c, 0x00, 0x00, 0x00, 0x26, 0x00, 0x01, 0x00, 0x00,
    0x00, 0x01, 0x00, 0x00, 0x49, 0xa0, 0x46, 0x05, 0x5f, 0x0f, 0x3c, 0xf5, 0x00, 0x03, 0x04, 0x00,
    0x00, 0x00, 0x00, 0x00, 0xda, 0x31, 0x91, 0x80, 0x00, 0x00, 0x00, 0x00, 0xda, 0x31, 0x91, 0x80,
    0x00, 0x32, 0x00, 0x00, 0x02, 0x4e, 0x02, 0xbc, 0x00, 0x00, 0x00, 0x03, 0x00, 0x02, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x03, 0x20, 0xff, 0x38, 0x00, 0x18, 0x02, 0x80,
    0x00, 0x32, 0x00, 0x32, 0x02, 0x4e, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x06,
    0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x02, 0x80, 0x01, 0x90, 0x00, 0x05,
    0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x3f, 0x3f, 0x3f, 0x3f, 0x00, 0x00, 0x00, 0x41, 0x00, 0x41, 0x03, 0x20, 0xff, 0x38,
    0x00, 0x18, 0x03, 0x20, 0x00, 0xc8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x02, 0x80, 0x00, 0x00, 0x00, 0x32, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x14, 0x00, 0x03, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x14, 0x00, 0x04, 0x00, 0x20, 0x00, 0x00, 0x00, 0x04, 0x00, 0x04, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x41, 0xff, 0xff, 0x00, 0x00, 0x00, 0x41, 0xff, 0xff, 0xff, 0xc0, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x00, 0x00, 0x00, 0x01, 0x00, 0x32,
    0x00, 0x00, 0x02, 0x4e, 0x02, 0xbc, 0x00, 0x05, 0x00, 0x00, 0x33, 0x01, 0x01, 0x23, 0x03, 0x03,
    0x32, 0x01, 0x0e, 0x01, 0x0e, 0x8c, 0x82, 0x8c, 0x02, 0xbc, 0xfd, 0x44, 0x01, 0x90, 0xfe, 0x70,
    0x00, 0x00, 0x00, 0x0a, 0x00, 0x7e, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x11,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x07, 0x00, 0x11, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x13, 0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x04, 0x00, 0x11, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x0f,
    0x00, 0x2b, 0x00, 0x03, 0x00, 0x01, 0x04, 0x09, 0x00, 0x01, 0x00, 0x22, 0x00, 0x3a, 0x00, 0x03,
    0x00, 0x01, 0x04, 0x09, 0x00, 0x02, 0x00, 0x0e, 0x00, 0x5c, 0x00, 0x03, 0x00, 0x01, 0x04, 0x09,
    0x00, 0x03, 0x00, 0x26, 0x00, 0x6a, 0x00, 0x03, 0x00, 0x01, 0x04, 0x09, 0x00, 0x04, 0x00, 0x22,
    0x00, 0x3a, 0x00, 0x03, 0x00, 0x01, 0x04, 0x09, 0x00, 0x06, 0x00, 0x1e, 0x00, 0x90, 0x4d, 0x61,
    0x73, 0x75, 0x20, 0x54, 0x65, 0x73, 0x74, 0x20, 0x4d, 0x65, 0x74, 0x72, 0x69, 0x63, 0x73, 0x52,
    0x65, 0x67, 0x75, 0x6c, 0x61, 0x72, 0x4d, 0x61, 0x73, 0x75, 0x20, 0x54, 0x65, 0x73, 0x74, 0x20,
    0x4d, 0x65, 0x74, 0x72, 0x69, 0x63, 0x73, 0x20, 0x31, 0x4d, 0x61, 0x73, 0x75, 0x54, 0x65, 0x73,
    0x74, 0x4d, 0x65, 0x74, 0x72, 0x69, 0x63, 0x73, 0x00, 0x4d, 0x00, 0x61, 0x00, 0x73, 0x00, 0x75,
    0x00, 0x20, 0x00, 0x54, 0x00, 0x65, 0x00, 0x73, 0x00, 0x74, 0x00, 0x20, 0x00, 0x4d, 0x00, 0x65,
    0x00, 0x74, 0x00, 0x72, 0x00, 0x69, 0x00, 0x63, 0x00, 0x73, 0x00, 0x52, 0x00, 0x65, 0x00, 0x67,
    0x00, 0x75, 0x00, 0x6c, 0x00, 0x61, 0x00, 0x72, 0x00, 0x4d, 0x00, 0x61, 0x00, 0x73, 0x00, 0x75,
    0x00, 0x20, 0x00, 0x54, 0x00, 0x65, 0x00, 0x73, 0x00, 0x74, 0x00, 0x20, 0x00, 0x4d, 0x00, 0x65,
    0x00, 0x74, 0x00, 0x72, 0x00, 0x69, 0x00, 0x63, 0x00, 0x73, 0x00, 0x20, 0x00, 0x31, 0x00, 0x4d,
    0x00, 0x61, 0x00, 0x73, 0x00, 0x75, 0x00, 0x54, 0x00, 0x65, 0x00, 0x73, 0x00, 0x74, 0x00, 0x4d,
    0x00, 0x65, 0x00, 0x74, 0x00, 0x72, 0x00, 0x69, 0x00, 0x63, 0x00, 0x73, 0x00, 0x02, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00,
    0x00, 0x24, 0x00, 0x00,
};

static void hexw(const WCHAR *s) { printf("["); for (; *s; s++) printf("%04x", *s); printf("]"); }

static void family(const WCHAR *name, GpFontCollection *collection) {
    GpFontFamily *f = NULL, *g = NULL;
    int s = GdipCreateFontFamilyFromName(name, collection, &f);
    printf("family ");
    hexw(name);
    printf(" %d", s);
    if (!s) {
        WCHAR n[LF_FACESIZE];
        n[0] = 0;
        GdipGetFamilyName(f, n, 0);
        printf(" name ");
        hexw(n);
        GdipGetFamilyName(f, n, 0x409);
        printf(" en ");
        hexw(n);
        GdipCreateFontFamilyFromName(name, collection, &g);
        printf(" same %d", f == g);
        UINT16 em = 0, a = 0, d = 0, ls = 0;
        GdipGetEmHeight(f, 0, &em);
        GdipGetCellAscent(f, 0, &a);
        GdipGetCellDescent(f, 0, &d);
        GdipGetLineSpacing(f, 0, &ls);
        printf("\n  metrics em %u asc %u desc %u spacing %u", em, a, d, ls);
        GdipDeleteFontFamily(f);
    }
    printf("\n");
}

static void outline_limits(HDC dc, int wide)
{
    UINT base = wide ? sizeof(OUTLINETEXTMETRICW) : sizeof(OUTLINETEXTMETRICA);
    UINT need = wide ? GetOutlineTextMetricsW(dc, 0, NULL) : GetOutlineTextMetricsA(dc, 0, NULL);
    BYTE *buffer = malloc(need + 1);
    UINT caps[] = {0, 1, base - 1, base, base + 1, need - 1};
    for (size_t i = 0; i < sizeof(caps) / sizeof(caps[0]); ++i) {
        UINT cap = caps[i];
        memset(buffer, 0xcc, need + 1);
        UINT result = wide ? GetOutlineTextMetricsW(dc, cap, (OUTLINETEXTMETRICW *)buffer)
                          : GetOutlineTextMetricsA(dc, cap, (OUTLINETEXTMETRICA *)buffer);
        UINT expected = cap == 0 && wide ? need : cap > base ? 0 : cap;
        printf("otm %c limit %zu result valid %d writes %d beyond unchanged %d",
               wide ? 'W' : 'A', i, result == expected, buffer[0] != 0xcc, buffer[cap] == 0xcc);
        if (cap == base) {
            OUTLINETEXTMETRICW *w = (OUTLINETEXTMETRICW *)buffer;
            OUTLINETEXTMETRICA *a = (OUTLINETEXTMETRICA *)buffer;
            printf(" names zero %d", wide ? !w->otmpFamilyName && !w->otmpFullName
                                          : !a->otmpFamilyName && !a->otmpFullName);
        }
        puts("");
    }
    free(buffer);
}

int main(void) {
    ULONG_PTR tok;
    StartupInput in = {1, NULL, FALSE, FALSE};
    GdiplusStartup(&tok, &in, NULL);
    GpFontFamily *f = NULL, *g = NULL;

    /* errors */
    printf("from name: NULL out %d NULL name %d\n", GdipCreateFontFamilyFromName(L"MS Gothic", NULL, NULL), GdipCreateFontFamilyFromName(NULL, NULL, &f));
    printf("delete NULL family %d font %d\n", GdipDeleteFontFamily(NULL), GdipDeleteFont(NULL));
    printf("name of NULL %d\n", GdipGetFamilyName(NULL, NULL, 0));
    printf("installed NULL %d\n", GdipNewInstalledFontCollection(NULL));

    GpFontCollection *private_fonts = NULL;
    int status = GdipNewPrivateFontCollection(&private_fonts);
    printf("private collection %d\n", status);
    if (status) return 1;
    status = GdipPrivateAddMemoryFont(private_fonts, test_font, sizeof(test_font));
    printf("private add memory %d\n", status);
    if (status) return 1;
    family(L"Masu Test Metrics", private_fonts);
    family(L"ThisFontShouldNotExist", private_fonts);
    GpStatus (WINAPI *gen[3])(GpFontFamily **) = {GdipGetGenericFontFamilySansSerif, GdipGetGenericFontFamilySerif, GdipGetGenericFontFamilyMonospace};
    for (int i = 0; i < 3; i++) {
        WCHAR n[LF_FACESIZE] = {0};
        int s = gen[i](&f);
        int name = s ? s : GdipGetFamilyName(f, n, 0);
        printf("generic %d: %d name %d nonempty %d\n", i, s, name, n[0] != 0);
    }

    /* the installed collection */
    GpFontCollection *c1, *c2;
    GdipNewInstalledFontCollection(&c1);
    GdipNewInstalledFontCollection(&c2);
    int n = 0, found = 0;
    GdipGetFontCollectionFamilyCount(c1, &n);
    printf("installed: same %d some %d\n", c1 == c2, n > 0);
    printf("list: 0 sought %d NULL list %d\n", GdipGetFontCollectionFamilyList(c1, 0, &f, &found), GdipGetFontCollectionFamilyList(c1, 1, NULL, &found));
    GpFontFamily *l1 = NULL, *l2 = NULL;
    GdipGetFontCollectionFamilyList(c1, 1, &l1, &found);
    GdipGetFontCollectionFamilyList(c1, 1, &l2, &found);
    printf("list: found %d same %d\n", found, l1 == l2);
    if (GdipCreateFontFamilyFromName(L"Masu Test Metrics", private_fonts, &f)) return 1;
    GpFontFamily *listed = NULL;
    int count = 0;
    GdipGetFontCollectionFamilyCount(private_fonts, &count);
    GdipGetFontCollectionFamilyList(private_fonts, 1, &listed, &found);
    printf("private: count %d found %d same family %d\n", count, found, f == listed);
    GdipCloneFontFamily(f, &g);
    printf("clone same %d\n", f == g);

    /* fonts from the family */
    HDC dc = CreateCompatibleDC(NULL);
    GpGraphics *gr;
    GdipCreateFromHDC(dc, &gr);
    UINT16 em = 0, ls = 0;
    GdipGetEmHeight(f, 0, &em);
    GdipGetLineSpacing(f, 0, &ls);
    for (int unit = 0; unit <= 7; unit++) {
        GpFont *font = NULL;
        int s = GdipCreateFont(f, 24, 0, unit, &font);
        printf("font unit %d: %d", unit, s);
        if (!s) {
            float size = 0, h = 0, h2 = 0, h3 = 0;
            int u = -1;
            GdipGetFontSize(font, &size);
            GdipGetFontUnit(font, &u);
            GdipGetFontHeightGivenDPI(font, 96, &h);
            GdipGetFontHeightGivenDPI(font, 192, &h2);
            GdipGetFontHeight(font, gr, &h3);
            LOGFONTW lf;
            memset(&lf, 0xcc, sizeof lf);
            GdipGetLogFontW(font, gr, &lf);
            /* the heights over the spacing per em (pixels at the DPI) */
            double k = (double)ls / em;
            printf(" size %.1f unit %d height96/k %.3f height192/k %.3f height/k %.3f lf %ld %ld %ld %ld %ld %d %d %d %d %d %d %d %d", size, u, h / k, h2 / k, h3 / k,
                   lf.lfHeight, lf.lfWidth, lf.lfEscapement, lf.lfOrientation, lf.lfWeight, lf.lfItalic, lf.lfUnderline, lf.lfStrikeOut,
                   lf.lfCharSet, lf.lfOutPrecision, lf.lfClipPrecision, lf.lfQuality, lf.lfPitchAndFamily);
            GdipDeleteFont(font);
        }
        printf("\n");
    }
    GpFont *font = NULL;
    printf("font: size 0 %d negative %d NULL family %d\n", GdipCreateFont(f, 0, 0, 2, &font), GdipCreateFont(f, -3, 0, 2, &font), GdipCreateFont(NULL, 12, 0, 2, &font));
    for (int style = 0; style < 16; style += 5) {
        int s = GdipCreateFont(f, 12, style, 2, &font);
        int st = -1;
        GdipGetFontStyle(font, &st);
        LOGFONTW lf;
        GdipGetLogFontW(font, gr, &lf);
        GpFontFamily *ff = NULL;
        GdipGetFamily(font, &ff);
        GpFont *cl = NULL;
        GdipCloneFont(font, &cl);
        float cs = 0;
        GdipGetFontSize(cl, &cs);
        printf("style %d: %d got %d weight %ld %d %d %d family same %d clone size %.1f\n", style, s, st, lf.lfWeight, lf.lfItalic, lf.lfUnderline, lf.lfStrikeOut, ff == f, cs);
        GdipDeleteFont(cl);
        GdipDeleteFont(font);
    }
    printf("logfont NULL graphics %d\n", (GdipCreateFont(f, 12, 0, 2, &font), GdipGetLogFontW(font, NULL, (LOGFONTW *)malloc(sizeof(LOGFONTW)))));
    GdipDeleteFont(font);

    /* Register the same face privately in GDI for LOGFONT and outline queries. */
    WCHAR temp[MAX_PATH], font_path[MAX_PATH];
    if (!GetTempPathW(MAX_PATH, temp) || !GetTempFileNameW(temp, L"mft", 0, font_path)) return 1;
    HANDLE file = CreateFileW(font_path, GENERIC_WRITE, 0, NULL, TRUNCATE_EXISTING, 0, NULL);
    DWORD bytes = 0;
    if (file != INVALID_HANDLE_VALUE) {
        WriteFile(file, test_font, sizeof(test_font), &bytes, NULL);
        CloseHandle(file);
    }
    int added = bytes == sizeof(test_font) && AddFontResourceExW(font_path, FR_PRIVATE, 0) > 0;
    printf("GDI private font %d\n", added);
    if (!added) {
        DeleteFileW(font_path);
        return 1;
    }

    /* LOGFONT constructors use an installed family. Heights that depend on its design
       metrics are tested as relationships. */
    static const struct { const WCHAR *face; int height, weight, charset; } lfs[] = {
        {L"Arial", -20, 400, ANSI_CHARSET},
        {L"Arial", 20, 400, ANSI_CHARSET},
        {L"Arial", 0, 400, ANSI_CHARSET},
        {L"ThisFontShouldNotExist", -20, 0, ANSI_CHARSET},
    };
    for (int i = 0; i < (int)(sizeof lfs / sizeof lfs[0]); i++) {
        LOGFONTW lf;
        memset(&lf, 0, sizeof lf);
        lstrcpyW(lf.lfFaceName, lfs[i].face);
        lf.lfHeight = lfs[i].height;
        lf.lfWeight = lfs[i].weight;
        lf.lfCharSet = lfs[i].charset;
        font = NULL;
        int s = GdipCreateFontFromLogfontW(dc, &lf, &font);
        printf("logfont %d: %d", i, s);
        if (!s) {
            float size = 0;
            int u = -1;
            GdipGetFontSize(font, &size);
            GdipGetFontUnit(font, &u);
            LOGFONTW o;
            GdipGetLogFontW(font, gr, &o);
            int sized = lfs[i].height < 0 ? size == -lfs[i].height
                       : lfs[i].height > 0 ? size > 0 && size <= lfs[i].height
                       : size > 0;
            printf(" unit %d size valid %d log height %d weight %ld charset %d name %d",
                   u, sized, o.lfHeight == -(LONG)(size + 0.5f), o.lfWeight,
                   o.lfCharSet, !lstrcmpW(o.lfFaceName, lfs[i].face));
            GdipDeleteFont(font);
        }
        printf("\n");
    }
    GdipDeleteGraphics(gr);

    /* GDI: the same family enumerated and its outline metrics */
    LOGFONTW q = { 0 };
    lstrcpyW(q.lfFaceName, L"Masu Test Metrics");
    q.lfCharSet = DEFAULT_CHARSET;
    struct found fd = { 0 };
    int er = EnumFontFamiliesExW(dc, &q, enum_tt, (LPARAM)&fd, 0);
    printf("enum private font: %d calls %d truetype %d em>0 %d\n", er, fd.calls, fd.truetype, fd.em > 0);
    q.lfHeight = -13;
    HFONT hf = CreateFontIndirectW(&q);
    HGDIOBJ old = SelectObject(dc, hf);
    UINT need = GetOutlineTextMetricsW(dc, 0, NULL);
    OUTLINETEXTMETRICW *otm = calloc(1, need + 16);
    UINT got = GetOutlineTextMetricsW(dc, need, otm);
    printf("otm: need>=sizeof %d got==need %d otmSize==need %d\n",
           need >= sizeof *otm, got == need, otm->otmSize == need);
    size_t fam = (size_t)otm->otmpFamilyName, full = (size_t)otm->otmpFullName;
    printf("otm names: family in %d after %d full in %d ascent==-height %d\n",
           fam > 0 && fam < need, fam >= sizeof *otm, full > 0 && full < need,
           otm->otmTextMetrics.tmAscent + otm->otmTextMetrics.tmDescent - otm->otmTextMetrics.tmInternalLeading == 13);
    outline_limits(dc, 1);
    outline_limits(dc, 0);
    free(otm);
    SelectObject(dc, old);
    DeleteObject(hf);
    DeleteDC(dc);
    RemoveFontResourceExW(font_path, FR_PRIVATE, 0);
    DeleteFileW(font_path);
    GdipDeleteFontFamily(f);
    if (g != f) GdipDeleteFontFamily(g);
    printf("private delete %d\n", GdipDeletePrivateFontCollection(&private_fonts));
    return 0;
}
