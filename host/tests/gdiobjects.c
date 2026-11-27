/* libs: -lgdi32 */
/* GDI recording handles preserve their object type through close, and ICM uses a stock color space. */
#include <windows.h>
#include <stdio.h>
#include <string.h>

#define CHECK(expr) do { if (!(expr)) { printf("failed at line %d\n", __LINE__); return 1; } } while (0)

static unsigned handle_type(HGDIOBJ h)
{
    return (unsigned)(((UINT_PTR)h >> 16) & 0x7f);
}

int main(void)
{
    HDC dc = CreateMetaFileW(NULL);
    CHECK(dc && handle_type(dc) == 0x66 && GetObjectType(dc) == OBJ_METADC);
    CHECK(!DeleteDC(dc));
    POINT before = {777, 888};
    CHECK(MoveToEx(dc, 2, 3, &before) && before.x == 777 && before.y == 888);
    CHECK(LineTo(dc, 10, 10));
    HMETAFILE wmf = CloseMetaFile(dc);
    CHECK(wmf && handle_type(wmf) == 0x26 && GetObjectType(wmf) == OBJ_METAFILE);
    BYTE bits[128];
    UINT n = GetMetaFileBitsEx(wmf, sizeof bits, bits);
    CHECK(n >= 44 && bits[0] == 1 && bits[2] == 9 && (bits[18] == 5 || bits[18] == 4));
    CHECK(!DeleteObject(wmf) && DeleteMetaFile(wmf));

    dc = CreateEnhMetaFileW(NULL, NULL, NULL, NULL);
    CHECK(dc && handle_type(dc) == 0x21 && GetObjectType(dc) == OBJ_ENHMETADC);
    HENHMETAFILE emf = CloseEnhMetaFile(dc);
    CHECK(emf && handle_type(emf) == 0x46 && GetObjectType(emf) == OBJ_ENHMETAFILE);
    ENHMETAHEADER hdr;
    CHECK(GetEnhMetaFileHeader(emf, sizeof hdr, &hdr) == sizeof hdr);
    CHECK(hdr.iType == EMR_HEADER && hdr.dSignature == ENHMETA_SIGNATURE && hdr.nRecords == 2);
    CHECK(hdr.nBytes == GetEnhMetaFileBits(emf, 0, NULL));
    CHECK(!DeleteObject(emf) && DeleteEnhMetaFile(emf));
    CHECK(DeleteDC(CreateEnhMetaFileW(NULL, NULL, NULL, NULL)));

    HDC view = CreateCompatibleDC(NULL);
    HCOLORSPACE stock = GetColorSpace(view);
    CHECK(stock && GetObjectType(stock) == OBJ_COLORSPACE && GetCurrentObject(view, OBJ_COLORSPACE) == stock);
    LOGCOLORSPACEA log;
    CHECK(GetLogColorSpaceA(stock, &log, sizeof log));
    CHECK(log.lcsSignature == 0x50534f43 && log.lcsSize == sizeof log && log.lcsCSType == 0x73524742);
    HCOLORSPACE copy = CreateColorSpaceA(&log);
    CHECK(copy && GetObjectType(copy) == OBJ_COLORSPACE);
    CHECK(SelectObject(view, copy) == stock && GetCurrentObject(view, OBJ_COLORSPACE) == copy);
    CHECK(!DeleteColorSpace(copy) && !DeleteObject(copy));
    CHECK(SetColorSpace(view, stock) == copy && DeleteColorSpace(copy));
    CHECK(DeleteDC(view));
    SetLastError(111);
    CHECK(GetColorSpace(NULL) == NULL && GetLastError() == ERROR_INVALID_PARAMETER);

    puts("GDI metafile objects and color spaces: ok");
    return 0;
}
