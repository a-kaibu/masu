/* AlphaBlend within one DC (found while fixing review 6852be1f's lock order: sake blended a
 * DC's overlapping rectangles from its live pixels). One 32-bit DIB section in one memory DC:
 * rectangles that overlap (each way, and touching edges that do not), the answer, the last
 * error, and whether a pixel changed.
 * libs: -lgdi32 -lmsimg32 */
#include "../probe.h"

static DWORD *bits;

static void blend(HDC dc, int dx, int dy, int sx, int sy) {
    BLENDFUNCTION bf = { AC_SRC_OVER, 0, 128, 0 };
    for (int i = 0; i < 64 * 64; i++) bits[i] = (i & 63) < 32 ? 0x00ff0000 : 0x000000ff;
    SetLastError(0xdead);
    BOOL r = AlphaBlend(dc, dx, dy, 20, 20, dc, sx, sy, 20, 20, bf);
    DWORD e = GetLastError();
    int changed = 0;
    for (int i = 0; i < 64 * 64; i++) changed += bits[i] != ((i & 63) < 32 ? 0x00ff0000u : 0x000000ffu);
    printf("dst (%d,%d) src (%d,%d): %d err %lu, %d pixels changed\n", dx, dy, sx, sy, r, e, changed);
}

int main(void) {
    BITMAPINFO bi = { { sizeof(BITMAPINFOHEADER), 64, -64, 1, 32, BI_RGB } };
    HDC dc = CreateCompatibleDC(NULL);
    HBITMAP b = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, (void **)&bits, NULL, 0);
    SelectObject(dc, b);
    blend(dc, 0, 0, 19, 19);
    blend(dc, 20, 20, 1, 1);
    blend(dc, 0, 0, 20, 10);
    blend(dc, 0, 0, 10, 20);
    blend(dc, 22, 0, 30, 5);
    blend(dc, 0, 0, 40, 40);
    // (a window origin moves both alike)
    SetWindowOrgEx(dc, -10, -10, NULL);
    blend(dc, 0, 0, 15, 15);
    blend(dc, 0, 0, 25, 0);
    DeleteDC(dc);
    DeleteObject(b);
    return 0;
}
