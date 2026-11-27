/* libs: -lstrmiids -lole32 */
#define COBJMACROS
#include <windows.h>
#include <dshow.h>
#include <stdio.h>

static void inspect_pin(IBaseFilter *filter, const WCHAR *id)
{
    IPin *pin = NULL;
    IEnumMediaTypes *types = NULL, *clone = NULL;
    HRESULT hr = IBaseFilter_FindPin(filter, id, &pin);
    if (FAILED(hr)) return;
    hr = IPin_EnumMediaTypes(pin, &types);
    printf("%ls enum %08lx\n", id, (unsigned long)hr);
    if (SUCCEEDED(hr)) {
        AM_MEDIA_TYPE *items[2] = {0};
        ULONG count = 999;
        hr = IEnumMediaTypes_Next(types, 1, items, &count);
        printf("next %08lx count %lu returned %d\n", (unsigned long)hr,
               count, !!items[0]);
        hr = IEnumMediaTypes_Next(types, 2, items, NULL);
        printf("next two without count %08lx\n", (unsigned long)hr);
        hr = IEnumMediaTypes_Clone(types, &clone);
        printf("clone %08lx\n", (unsigned long)hr);
        if (clone) {
            hr = IEnumMediaTypes_Skip(clone, 1);
            printf("skip %08lx\n", (unsigned long)hr);
            hr = IEnumMediaTypes_Reset(clone);
            printf("reset %08lx\n", (unsigned long)hr);
            IEnumMediaTypes_Release(clone);
        }
        IEnumMediaTypes_Release(types);
    }
    IPin_Release(pin);
}

int main(void)
{
    if (FAILED(CoInitializeEx(NULL, COINIT_APARTMENTTHREADED))) return 2;
    const CLSID *decoders[] = {&CLSID_CMpegAudioCodec, &CLSID_CMpegVideoCodec};
    for (unsigned d = 0; d < sizeof(decoders) / sizeof(decoders[0]); ++d) {
        IBaseFilter *filter = NULL;
        HRESULT hr = CoCreateInstance(decoders[d], NULL, CLSCTX_INPROC_SERVER,
                                      &IID_IBaseFilter, (void **)&filter);
        if (FAILED(hr)) return 1;
        printf("decoder %u\n", d);
        inspect_pin(filter, L"In");
        inspect_pin(filter, L"Out");
        IBaseFilter_Release(filter);
    }
    CoUninitialize();
    return 0;
}
