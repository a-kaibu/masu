/* IDirect3DDevice9Ex's own calls on a windowed device: Get/SetGPUThreadPriority (each value
 * from -8 to 8), Get/SetMaximumFrameLatency (0, 1, 16, 20, 21, 30, 31), WaitForVBlank (the
 * implicit swap chain and past it; GetRasterStatus right after), CheckResourceResidency
 * (resources of the device, none, past 65535), CheckDeviceState (its window, NULL, another),
 * SetConvolutionMonoKernel (sizes and NULL weights), ComposeRects without its surfaces, and
 * whether A1 surfaces exist (CheckDeviceFormat).
 * libs: -ld3d9
 * arch: 32 */
#define COBJMACROS
#include "../probe.h"
#include <d3d9.h>

typedef HRESULT(WINAPI *create9ex)(UINT, IDirect3D9Ex **);

int main(void) {
    WNDCLASSA wc = { 0, DefWindowProcA, 0, 0, GetModuleHandleA(NULL), 0, 0, 0, 0, "probe" };
    RegisterClassA(&wc);
    HWND hwnd = CreateWindowA("probe", "d3d9ex", WS_POPUP, 0, 0, 64, 64, 0, 0, wc.hInstance, 0);
    HWND other = CreateWindowA("probe", "other", WS_POPUP, 0, 0, 16, 16, 0, 0, wc.hInstance, 0);
    create9ex f = (create9ex)GetProcAddress(LoadLibraryA("d3d9.dll"), "Direct3DCreate9Ex");
    IDirect3D9Ex *d3d = NULL;
    if (!f || FAILED(f(D3D_SDK_VERSION, &d3d))) { puts("skip no Direct3DCreate9Ex"); return 0; }
    D3DPRESENT_PARAMETERS pp = { 0 };
    pp.BackBufferWidth = 64; pp.BackBufferHeight = 64; pp.BackBufferFormat = D3DFMT_X8R8G8B8; pp.BackBufferCount = 1;
    pp.SwapEffect = D3DSWAPEFFECT_DISCARD; pp.hDeviceWindow = hwnd; pp.Windowed = TRUE;
    IDirect3DDevice9Ex *dev = NULL;
    HRESULT hr = IDirect3D9Ex_CreateDeviceEx(d3d, D3DADAPTER_DEFAULT, D3DDEVTYPE_HAL, hwnd, D3DCREATE_SOFTWARE_VERTEXPROCESSING, &pp, NULL, &dev);
    if (FAILED(hr)) { printf("skip CreateDeviceEx %#lx\n", hr); return 0; }

    INT prio = 99;
    hr = IDirect3DDevice9Ex_GetGPUThreadPriority(dev, &prio);
    printf("GetGPUThreadPriority %#lx %d\n", hr, prio);
    for (int p = -8; p <= 8; p++) {
        HRESULT s = IDirect3DDevice9Ex_SetGPUThreadPriority(dev, p);
        prio = 99;
        hr = IDirect3DDevice9Ex_GetGPUThreadPriority(dev, &prio);
        printf("SetGPUThreadPriority %d %#lx get %#lx %d\n", p, s, hr, prio);
    }
    IDirect3DDevice9Ex_SetGPUThreadPriority(dev, 0);

    UINT lat = 99;
    hr = IDirect3DDevice9Ex_GetMaximumFrameLatency(dev, &lat);
    printf("GetMaximumFrameLatency %#lx %u\n", hr, lat);
    static const UINT lats[] = { 1, 0, 16, 20, 21, 30, 31, 0xffffffff, 2 };
    for (int i = 0; i < (int)(sizeof lats / sizeof lats[0]); i++) {
        HRESULT s = IDirect3DDevice9Ex_SetMaximumFrameLatency(dev, lats[i]);
        lat = 99;
        hr = IDirect3DDevice9Ex_GetMaximumFrameLatency(dev, &lat);
        printf("SetMaximumFrameLatency %u %#lx get %#lx %u\n", lats[i], s, hr, lat);
    }

    D3DRASTER_STATUS rs;
    for (int i = 0; i < 3; i++) {
        hr = IDirect3DDevice9Ex_WaitForVBlank(dev, 0);
        IDirect3DDevice9Ex_GetRasterStatus(dev, 0, &rs);
        printf("WaitForVBlank 0 %#lx\n", hr);
    }
    printf("WaitForVBlank 1 %#lx\n", IDirect3DDevice9Ex_WaitForVBlank(dev, 1));

    IDirect3DTexture9 *t = NULL;
    IDirect3DVertexBuffer9 *vb = NULL;
    IDirect3DSurface9 *rt = NULL;
    IDirect3DDevice9Ex_CreateTexture(dev, 4, 4, 1, 0, D3DFMT_A8R8G8B8, D3DPOOL_DEFAULT, &t, NULL);
    IDirect3DDevice9Ex_CreateVertexBuffer(dev, 64, 0, 0, D3DPOOL_DEFAULT, &vb, NULL);
    IDirect3DDevice9Ex_CreateRenderTarget(dev, 4, 4, D3DFMT_A8R8G8B8, D3DMULTISAMPLE_NONE, 0, FALSE, &rt, NULL);
    IDirect3DResource9 *list[3] = { (IDirect3DResource9 *)t, (IDirect3DResource9 *)vb, (IDirect3DResource9 *)rt };
    printf("CheckResourceResidency 3 %#lx\n", IDirect3DDevice9Ex_CheckResourceResidency(dev, list, 3));
    printf("CheckResourceResidency 0 %#lx\n", IDirect3DDevice9Ex_CheckResourceResidency(dev, NULL, 0));
    printf("CheckResourceResidency NULL 1 %#lx\n", IDirect3DDevice9Ex_CheckResourceResidency(dev, NULL, 1));
    static IDirect3DResource9 *big[65536];
    for (int i = 0; i < 65536; i++) big[i] = (IDirect3DResource9 *)t;
    printf("CheckResourceResidency 65535 %#lx\n", IDirect3DDevice9Ex_CheckResourceResidency(dev, big, 65535));
    printf("CheckResourceResidency 65536 %#lx\n", IDirect3DDevice9Ex_CheckResourceResidency(dev, big, 65536));

    printf("CheckDeviceState own %#lx NULL %#lx other %#lx\n", IDirect3DDevice9Ex_CheckDeviceState(dev, hwnd), IDirect3DDevice9Ex_CheckDeviceState(dev, NULL),
           IDirect3DDevice9Ex_CheckDeviceState(dev, other));

    float w[8] = { 1, 2, 3, 4, 5, 6, 7, 8 };
    static const UINT ks[][2] = { { 1, 1 }, { 7, 7 }, { 8, 1 }, { 1, 8 }, { 0, 0 }, { 3, 5 } };
    for (int i = 0; i < (int)(sizeof ks / sizeof ks[0]); i++)
        printf("SetConvolutionMonoKernel %ux%u %#lx NULL %#lx\n", ks[i][0], ks[i][1], IDirect3DDevice9Ex_SetConvolutionMonoKernel(dev, ks[i][0], ks[i][1], w, w),
               IDirect3DDevice9Ex_SetConvolutionMonoKernel(dev, ks[i][0], ks[i][1], NULL, NULL));
    printf("ComposeRects NULL %#lx\n", IDirect3DDevice9Ex_ComposeRects(dev, NULL, NULL, NULL, 0, NULL, D3DCOMPOSERECTS_COPY, 0, 0));
    printf("A1 surface %d texture %d\n", IDirect3D9Ex_CheckDeviceFormat(d3d, 0, D3DDEVTYPE_HAL, D3DFMT_X8R8G8B8, 0, D3DRTYPE_SURFACE, D3DFMT_A1) == D3D_OK,
           IDirect3D9Ex_CheckDeviceFormat(d3d, 0, D3DDEVTYPE_HAL, D3DFMT_X8R8G8B8, 0, D3DRTYPE_TEXTURE, D3DFMT_A1) == D3D_OK);
    IDirect3DSurface9 *a1 = NULL;
    hr = IDirect3DDevice9Ex_CreateOffscreenPlainSurface(dev, 64, 64, D3DFMT_A1, D3DPOOL_DEFAULT, &a1, NULL);
    printf("CreateOffscreenPlainSurface A1 %#lx\n", hr);
    if (a1) IDirect3DSurface9_Release(a1);
    IDirect3DSurface9_Release(rt);
    IDirect3DVertexBuffer9_Release(vb);
    IDirect3DTexture9_Release(t);
    IDirect3DDevice9Ex_Release(dev);
    IDirect3D9Ex_Release(d3d);
    return 0;
}
