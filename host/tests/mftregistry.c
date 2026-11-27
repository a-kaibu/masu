/* libs: -lmfplat -lmfuuid -lole32 -ladvapi32 */
#define COBJMACROS
#define _WIN32_WINNT 0x0602
#include <windows.h>
#include <mfapi.h>
#include <mftransform.h>
#include <stdio.h>
#include <string.h>
#include <wchar.h>

static const GUID transform = {0x95fc443a,0xb756,0x436c,{0x9a,1,2,0x54,0x35,6,0x89,8}};
static const GUID category = {0x3415f040,0x63eb,0x47e9,{0xba,9,0xa2,0x1a,8,0x25,0x72,3}};
static const GUID counter = {0x543f6e75,0xad04,0x4cd3,{0x8d,3,1,0x58,0x78,0x80,0xda,0x68}};
static MFT_REGISTER_TYPE_INFO input = {{3,4,5,{6}},{7,8,9,{10}}};
static MFT_REGISTER_TYPE_INFO output = {{11,12,13,{14}},{15,16,17,{18}}};

static int run(void)
{
    UINT32 written = 0;
    IMFAttributes *initial = NULL;
    HRESULT hr = MFCreateAttributes(&initial, 1);
    if (FAILED(hr)) return 2;
    static const BYTE original_blob[] = {0, 1, 0xfc, 0xff};
    IMFAttributes_SetUINT32(initial, &counter, 57);
    IMFAttributes_SetGUID(initial, &category, &transform);
    IMFAttributes_SetString(initial, &transform, L"Text");
    IMFAttributes_SetBlob(initial, &input.guidMajorType, original_blob, sizeof(original_blob));
    IMFAttributes_SetDouble(initial, &input.guidSubtype, 1.25);
    IMFAttributes_SetUINT64(initial, &output.guidSubtype, 0x1111222233334444ULL);
    hr = MFTRegister(transform, category, L"Masu transform probe", 1, 1, &input, 1, &output, initial);
    IMFAttributes_Release(initial);
    printf("register %08lx\n", (unsigned long)hr);
    if (FAILED(hr)) return 2;

    /* Supplying no new type array leaves the registered formats intact. */
    hr = MFTRegister(transform, category, L"Masu transform probe", 0, 1, NULL, 0, NULL, NULL);
    if (FAILED(hr)) return 3;

    LPWSTR name = NULL;
    MFT_REGISTER_TYPE_INFO *ins = NULL, *outs = NULL;
    UINT32 ni = 0, no = 0;
    IMFAttributes *attrs = NULL;
    hr = MFTGetInfo(transform, &name, &ins, &ni, &outs, &no, &attrs);
    if (attrs) IMFAttributes_GetUINT32(attrs, &counter, &written);
    printf("info %08lx name %d inputs %u outputs %u types %d %d attrs %d value %u\n",
           (unsigned long)hr, name && wcscmp(name, L"Masu transform probe") == 0, ni, no,
           ni == 1 && ins && !memcmp(ins, &input, sizeof(input)),
           no == 1 && outs && !memcmp(outs, &output, sizeof(output)), !!attrs, written);
    CoTaskMemFree(name);
    CoTaskMemFree(ins);
    CoTaskMemFree(outs);
    GUID id = {0};
    WCHAR text[5] = {0};
    BYTE bytes[4] = {0};
    double ratio = 0;
    UINT64 wide = 0;
    if (attrs) {
        IMFAttributes_GetGUID(attrs, &category, &id);
        IMFAttributes_GetString(attrs, &transform, text, 5, NULL);
        IMFAttributes_GetBlob(attrs, &input.guidMajorType, bytes, sizeof(bytes), NULL);
        IMFAttributes_GetDouble(attrs, &input.guidSubtype, &ratio);
        IMFAttributes_GetUINT64(attrs, &output.guidSubtype, &wide);
        IMFAttributes_Release(attrs);
    }
    printf("attribute types %d %d %d %d %d\n",
           !memcmp(&id, &transform, sizeof(id)), !wcscmp(text, L"Text"),
           !memcmp(bytes, original_blob, sizeof(bytes)), ratio == 1.25,
           wide == 0x1111222233334444ULL);
    ni = no = 999;
    hr = MFTGetInfo(transform, NULL, NULL, &ni, NULL, &no, NULL);
    printf("counts without arrays %08lx %u %u\n", (unsigned long)hr, ni, no);

    for (int i = 0; i < 2; ++i) {
        CLSID *ids = NULL;
        UINT32 count = 999;
        hr = MFTEnum(category, 0, i ? &output : &input, i ? &input : &output, NULL, &ids, &count);
        printf("enum %d %08lx count %u registered %d\n", i, (unsigned long)hr, count,
               count == 1 && ids && !memcmp(ids, &transform, sizeof(transform)));
        CoTaskMemFree(ids);
    }
    hr = MFTRegister(transform, category, L"Masu transform probe", MFT_ENUM_FLAG_ASYNCMFT,
                     0, NULL, 0, NULL, NULL);
    printf("register async %08lx\n", (unsigned long)hr);
    hr = MFTRegister(transform, category, L"Masu transform probe", 0,
                     0, NULL, 0, NULL, NULL);
    printf("reregister without flags %08lx\n", (unsigned long)hr);
    CLSID *async_ids = NULL;
    UINT32 async_count = 999;
    hr = MFTEnum(category, 0, &input, &output, NULL, &async_ids, &async_count);
    printf("enum sync only %08lx count %u pointer %d\n",
           (unsigned long)hr, async_count, !!async_ids);
    CoTaskMemFree(async_ids);

    hr = MFTUnregister(transform);
    printf("unregister %08lx\n", (unsigned long)hr);
    hr = MFTUnregister(transform);
    printf("unregister again %08lx\n", (unsigned long)hr);
    CLSID *missing_ids = NULL;
    UINT32 missing_count = 999;
    hr = MFTEnum(category, 0, NULL, NULL, NULL, &missing_ids, &missing_count);
    printf("enum after unregister %08lx count %u pointer %d\n",
           (unsigned long)hr, missing_count, !!missing_ids);
    CoTaskMemFree(missing_ids);
    hr = MFTGetInfo(transform, NULL, NULL, NULL, NULL, NULL, NULL);
    printf("missing %08lx\n", (unsigned long)hr);
    return 0;
}

int main(int argc, char **argv)
{
    (void)argv;
    HKEY root = NULL;
    WCHAR key[128];
    /* Use an isolated registry when capturing Windows' output. The host runner uses sake. */
    if (argc > 1) {
        swprintf(key, sizeof(key) / sizeof(*key), L"Software\\MasuMftProbe_%lu_%llu",
                 (unsigned long)GetCurrentProcessId(), (unsigned long long)GetTickCount64());
        if (RegCreateKeyExW(HKEY_CURRENT_USER, key, 0, NULL, REG_OPTION_NON_VOLATILE,
                           KEY_ALL_ACCESS, NULL, &root, NULL)) return 4;
        if (RegOverridePredefKey(HKEY_CLASSES_ROOT, root) ||
            RegOverridePredefKey(HKEY_LOCAL_MACHINE, root)) {
            RegOverridePredefKey(HKEY_CLASSES_ROOT, NULL);
            RegCloseKey(root);
            RegDeleteTreeW(HKEY_CURRENT_USER, key);
            return 5;
        }
    }
    const int result = run();
    if (root) {
        RegOverridePredefKey(HKEY_CLASSES_ROOT, NULL);
        RegOverridePredefKey(HKEY_LOCAL_MACHINE, NULL);
        RegCloseKey(root);
        RegDeleteTreeW(HKEY_CURRENT_USER, key);
    }
    return result;
}
