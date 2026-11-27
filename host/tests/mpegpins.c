/* libs: -lstrmiids -lole32 */
#define COBJMACROS
#include <windows.h>
#include <dshow.h>
#include <stdio.h>
#include <wchar.h>

static int inspect(const char *decoder, const CLSID *clsid)
{
    IBaseFilter *filter = NULL;
    HRESULT hr = CoCreateInstance(clsid, NULL, CLSCTX_INPROC_SERVER,
                                  &IID_IBaseFilter, (void **)&filter);
    printf("%s create %08lx\n", decoder, (unsigned long)hr);
    if (FAILED(hr)) return 1;

    const WCHAR *lookups[] = {L"In", L"Out", L"XForm In", L"XForm Out", L"Input", L"Output", L"in"};
    for (unsigned i = 0; i < sizeof(lookups) / sizeof(lookups[0]); ++i) {
        IPin *pin = NULL;
        hr = IBaseFilter_FindPin(filter, lookups[i], &pin);
        printf("%s lookup %ls %08lx", decoder, lookups[i], (unsigned long)hr);
        if (SUCCEEDED(hr) && pin) {
            LPWSTR id = NULL;
            PIN_INFO info = {0};
            HRESULT id_hr = IPin_QueryId(pin, &id);
            HRESULT info_hr = IPin_QueryPinInfo(pin, &info);
            if (SUCCEEDED(id_hr) && SUCCEEDED(info_hr)) {
                printf(" id %ls name %ls direction %d same owner %d",
                       id, info.achName, info.dir, info.pFilter == filter);
            }
            if (info.pFilter) IBaseFilter_Release(info.pFilter);
            CoTaskMemFree(id);
            IPin_Release(pin);
        }
        puts("");
    }
    IBaseFilter_Release(filter);
    return 0;
}

int main(void)
{
    if (FAILED(CoInitializeEx(NULL, COINIT_APARTMENTTHREADED))) return 2;
    int errors = inspect("audio", &CLSID_CMpegAudioCodec);
    errors += inspect("video", &CLSID_CMpegVideoCodec);
    CoUninitialize();
    return errors;
}
