/* libs: -lstrmiids -lole32 */
#define COBJMACROS
#include <windows.h>
#include <dshow.h>
#include <stdio.h>

/* MinGW declares the decoder IID but omits the interface. */
struct decoder {
    const struct decoder_vtable *vtable;
};
struct decoder_option {
    HRESULT (WINAPI *get)(struct decoder *, ULONG *);
    HRESULT (WINAPI *put)(struct decoder *, ULONG);
};
struct decoder_vtable {
    HRESULT (WINAPI *query)(struct decoder *, REFIID, void **);
    ULONG (WINAPI *retain)(struct decoder *);
    ULONG (WINAPI *release)(struct decoder *);
    struct decoder_option option[6];
};

int main(void)
{
    if (FAILED(CoInitializeEx(NULL, COINIT_APARTMENTTHREADED))) return 1;
    IBaseFilter *filter = NULL;
    if (FAILED(CoCreateInstance(&CLSID_CMpegAudioCodec, NULL, CLSCTX_INPROC_SERVER,
                               &IID_IBaseFilter, (void **)&filter))) return 2;
    struct decoder *decoder = NULL;
    HRESULT hr = IBaseFilter_QueryInterface(filter, &IID_IMpegAudioDecoder, (void **)&decoder);
    printf("decoder %08lx\n", (unsigned long)hr);
    if (FAILED(hr)) return 2;

    IUnknown *identity = NULL;
    hr = decoder->vtable->query(decoder, &IID_IUnknown, (void **)&identity);
    IUnknown *filter_identity = NULL;
    IBaseFilter_QueryInterface(filter, &IID_IUnknown, (void **)&filter_identity);
    printf("identity %08lx same object %d\n", (unsigned long)hr, identity && identity == filter_identity);
    if (filter_identity) IUnknown_Release(filter_identity);
    if (identity) IUnknown_Release(identity);

    const ULONG values[] = {2, 0, 1, 8, 1, 2};
    for (unsigned i = 0; i < sizeof(values) / sizeof(values[0]); ++i) {
        const struct decoder_option *option = &decoder->vtable->option[i];
        ULONG initial = 999, current = 999;
        HRESULT read = option->get(decoder, &initial);
        HRESULT allowed = option->put(decoder, values[i]);
        HRESULT invalid = option->put(decoder, 255);
        HRESULT reread = option->get(decoder, &current);
        printf("option %u read %08lx %lu accepted %08lx invalid %08lx reread %08lx %lu\n",
               i, (unsigned long)read, initial, (unsigned long)allowed, (unsigned long)invalid,
               (unsigned long)reread, current);
    }

    ULONG remaining = decoder->vtable->release(decoder);
    ULONG final = IBaseFilter_Release(filter);
    printf("release %lu %lu\n", remaining, final);
    CoUninitialize();
    return 0;
}
