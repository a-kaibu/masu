/* libs: -luxtheme -lgdi32 */
#define _WIN32_WINNT 0x0600
#include <windows.h>
#include <uxtheme.h>
#include <stdio.h>
#include <string.h>

int main(void)
{
    HDC target = CreateCompatibleDC(NULL);
    BITMAPINFO info = {0};
    info.bmiHeader.biSize = sizeof(info.bmiHeader);
    info.bmiHeader.biWidth = 32;
    info.bmiHeader.biHeight = -32;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    void *target_bits = NULL;
    HBITMAP canvas = CreateDIBSection(target, &info, DIB_RGB_COLORS, &target_bits, NULL, 0);
    HGDIOBJ previous = SelectObject(target, canvas);
    printf("init %08lx\n", (unsigned long)BufferedPaintInit());

    RECT region = {7, 9, 12, 15};
    RECT invalid = {0, 0, 0, 10};
    HDC paint = (HDC)(ULONG_PTR)7;
    HPAINTBUFFER buffer = BeginBufferedPaint(target, &invalid, BPBF_DIB, NULL, &paint);
    printf("empty buffer %d dc %d\n", !!buffer, !!paint);
    for (unsigned format = BPBF_COMPATIBLEBITMAP; format <= BPBF_TOPDOWNMONODIB; ++format) {
        paint = NULL;
        BP_PAINTPARAMS params = {sizeof(params), BPPF_ERASE, NULL, NULL};
        buffer = BeginBufferedPaint(target, &region, format, &params, &paint);
        printf("format %u buffer %d dc %d target %d", format, !!buffer, !!paint,
               buffer && GetBufferedPaintTargetDC(buffer) == target);
        if (!buffer) { puts(""); continue; }

        RECT actual = {0};
        HRESULT rect = GetBufferedPaintTargetRect(buffer, &actual);
        POINT origin = {0};
        GetWindowOrgEx(paint, &origin);
        RGBQUAD *pixels = (RGBQUAD *)(ULONG_PTR)12;
        int width = -1;
        HRESULT bits = GetBufferedPaintBits(buffer, &pixels, &width);
        BITMAP bitmap = {0};
        GetObjectW(GetCurrentObject(paint, OBJ_BITMAP), sizeof(bitmap), &bitmap);
        printf(" region %08lx %d origin %ld %ld bits %08lx %d width %d bpp %u\n",
               (unsigned long)rect, EqualRect(&actual, &region), origin.x, origin.y,
               (unsigned long)bits, pixels != NULL && (ULONG_PTR)pixels != 12, width >= 5, bitmap.bmBitsPixel);
        HRESULT clear = BufferedPaintClear(buffer, NULL);
        printf("clear %08lx end %08lx\n", (unsigned long)clear, (unsigned long)EndBufferedPaint(buffer, FALSE));
    }

    COLORREF before = RGB(25, 50, 75);
    COLORREF pen = RGB(200, 40, 60);
    SetPixel(target, 8, 10, before);
    buffer = BeginBufferedPaint(target, &region, BPBF_TOPDOWNDIB, NULL, &paint);
    if (buffer) {
        SetPixel(paint, 8, 10, pen);
        HRESULT result = EndBufferedPaint(buffer, FALSE);
        printf("discard %08lx %d\n", (unsigned long)result, GetPixel(target, 8, 10) == before);
    }
    buffer = BeginBufferedPaint(target, &region, BPBF_TOPDOWNDIB, NULL, &paint);
    if (buffer) {
        SetPixel(paint, 8, 10, pen);
        HRESULT result = EndBufferedPaint(buffer, TRUE);
        printf("commit %08lx %d\n", (unsigned long)result, GetPixel(target, 8, 10) == pen);
    }

    RGBQUAD *sentinel = (RGBQUAD *)(ULONG_PTR)12;
    int width = -1;
    HRESULT result = GetBufferedPaintBits(NULL, &sentinel, &width);
    printf("invalid bits %08lx unchanged %d %d\n", (unsigned long)result,
           (ULONG_PTR)sentinel == 12, width == -1);
    SelectObject(target, previous);
    DeleteObject(canvas);
    DeleteDC(target);
    printf("uninit %08lx\n", (unsigned long)BufferedPaintUnInit());
    return 0;
}
