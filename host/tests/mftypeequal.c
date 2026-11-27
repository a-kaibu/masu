/* libs: -lmfplat -lmfuuid -lole32 */
#define COBJMACROS
#define _WIN32_WINNT 0x0602
#include <windows.h>
#include <mfapi.h>
#include <mfidl.h>
#include <stdio.h>

static const GUID key_a = {0x9d6870aa,0x0512,0x8877,{3,4,5,6,7,8,9,10}};
static const GUID key_b = {0xd8808d9a,0x3dd2,0x9aa7,{3,4,5,6,7,8,9,10}};

static void compare(const char *name, IMFMediaType *a, IMFMediaType *b)
{
    DWORD flags = 0xdeadbeef;
    HRESULT hr = IMFMediaType_IsEqual(a, b, &flags);
    printf("%s %08lx flags %lx\n", name, (unsigned long)hr, (unsigned long)flags);
}

int main(void)
{
    IMFMediaType *a = NULL, *b = NULL;
    if (FAILED(MFCreateMediaType(&a)) || FAILED(MFCreateMediaType(&b))) return 1;
    compare("no major types", a, b);

    IMFMediaType_SetGUID(a, &MF_MT_MAJOR_TYPE, &MFMediaType_Video);
    IMFMediaType_SetGUID(b, &MF_MT_MAJOR_TYPE, &MFMediaType_Audio);
    compare("different major types", a, b);
    IMFMediaType_SetGUID(b, &MF_MT_MAJOR_TYPE, &MFMediaType_Video);
    compare("both subtypes absent", a, b);

    IMFMediaType_SetGUID(a, &MF_MT_SUBTYPE, &MFVideoFormat_RGB32);
    compare("one subtype absent", a, b);
    IMFMediaType_SetGUID(b, &MF_MT_SUBTYPE, &MFVideoFormat_RGB565);
    compare("different subtypes", a, b);
    IMFMediaType_SetGUID(b, &MF_MT_SUBTYPE, &MFVideoFormat_RGB32);
    IMFMediaType_SetUINT32(a, &key_a, 13);
    compare("extra format attribute", a, b);
    compare("extra format reversed", b, a);
    IMFMediaType_SetUINT32(b, &key_b, 12);
    compare("different format keys", a, b);
    IMFMediaType_DeleteItem(b, &key_b);
    IMFMediaType_SetUINT32(b, &key_a, 14);
    compare("different format value", a, b);
    IMFMediaType_SetUINT32(b, &key_a, 13);
    IMFMediaType_SetUINT64(a, &MF_MT_FRAME_RATE_RANGE_MIN, 0x0000001e00000001ULL);
    IMFMediaType_SetUINT64(b, &MF_MT_FRAME_RATE_RANGE_MIN, 0x0000000f00000001ULL);
    compare("different minimum frame rate", a, b);
    IMFMediaType_SetUINT64(a, &MF_MT_FRAME_RATE_RANGE_MAX, 0x0000003c00000001ULL);
    compare("extra maximum frame rate", a, b);

    const BYTE data_a[] = {2, 3, 6};
    const BYTE data_b[] = {2, 3, 7};
    IMFMediaType_SetBlob(a, &MF_MT_USER_DATA, data_a, sizeof(data_a));
    compare("extra user data", a, b);
    IMFMediaType_SetBlob(b, &MF_MT_USER_DATA, data_b, sizeof(data_b));
    compare("different user data", a, b);
    IMFMediaType_SetBlob(b, &MF_MT_USER_DATA, data_a, sizeof(data_a));
    compare("matching user data", a, b);

    IMFMediaType_Release(b);
    IMFMediaType_Release(a);
    return 0;
}
