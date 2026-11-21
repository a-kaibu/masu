/* Direct3D 11 clears of views by value: ClearUnorderedAccessViewUint and
 * ClearUnorderedAccessViewFloat of 8x4 textures of several formats (values past a format's
 * range, NaN, a Float clear of an integer format, a Uint clear of a float one) and
 * ID3D11DeviceContext1::ClearView of render target and unordered access views (rectangles
 * inside, partly outside, empty and inverted ones; NULL rectangles; an sRGB target), each
 * texture's texels read back after each (every texel, its bytes from the highest).
 * WARP or hardware on the Windows runner.
 * libs: -ld3d11
 * run: browser (sake's Direct3D 11 runs in the page: tests/winref/run-browser.mjs) */
#define COBJMACROS
#define INITGUID
#include "../probe.h"
#include <d3d11_1.h>
#include <math.h>

#define W 8
#define H 4

static ID3D11Device *dev;
static ID3D11DeviceContext *ctx;
static ID3D11DeviceContext1 *ctx1;

static const struct { DXGI_FORMAT f; const char *n; int bpp; } fm[] = {
    { DXGI_FORMAT_R8G8B8A8_UNORM, "R8G8B8A8_UNORM", 4 }, { DXGI_FORMAT_R8G8B8A8_UINT, "R8G8B8A8_UINT", 4 },
    { DXGI_FORMAT_R8G8B8A8_SNORM, "R8G8B8A8_SNORM", 4 }, { DXGI_FORMAT_R16G16_FLOAT, "R16G16_FLOAT", 4 },
    { DXGI_FORMAT_R16G16_UNORM, "R16G16_UNORM", 4 }, { DXGI_FORMAT_R16G16_SNORM, "R16G16_SNORM", 4 },
    { DXGI_FORMAT_R32_FLOAT, "R32_FLOAT", 4 }, { DXGI_FORMAT_R32_UINT, "R32_UINT", 4 }, { DXGI_FORMAT_R32_SINT, "R32_SINT", 4 },
    { DXGI_FORMAT_R10G10B10A2_UNORM, "R10G10B10A2_UNORM", 4 }, { DXGI_FORMAT_R11G11B10_FLOAT, "R11G11B10_FLOAT", 4 },
    { DXGI_FORMAT_R8_UNORM, "R8_UNORM", 1 }, { DXGI_FORMAT_R16G16B16A16_FLOAT, "R16G16B16A16_FLOAT", 8 },
    { DXGI_FORMAT_R32G32B32A32_FLOAT, "R32G32B32A32_FLOAT", 16 }, { DXGI_FORMAT_R32G32B32A32_UINT, "R32G32B32A32_UINT", 16 },
    { DXGI_FORMAT_B8G8R8A8_UNORM, "B8G8R8A8_UNORM", 4 },
};

static ID3D11Texture2D *make(DXGI_FORMAT f, UINT bind) {
    D3D11_TEXTURE2D_DESC d = { W, H, 1, 1, f, { 1, 0 }, D3D11_USAGE_DEFAULT, bind, 0, 0 };
    static unsigned char zero[W * H * 16];
    for (int i = 0; i < (int)sizeof zero; i++) zero[i] = (unsigned char)(i * 7 + 3);
    D3D11_SUBRESOURCE_DATA init = { zero, W * 16, 0 };
    ID3D11Texture2D *t = NULL;
    HRESULT hr = ID3D11Device_CreateTexture2D(dev, &d, &init, &t);
    if (FAILED(hr)) printf("  CreateTexture2D %#lx\n", hr);
    return t;
}

/* every texel's bytes, one row per line */
static void dump(ID3D11Texture2D *t, DXGI_FORMAT f, int bpp) {
    D3D11_TEXTURE2D_DESC d = { W, H, 1, 1, f, { 1, 0 }, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ, 0 };
    ID3D11Texture2D *s = NULL;
    if (FAILED(ID3D11Device_CreateTexture2D(dev, &d, NULL, &s))) { printf("  staging failed\n"); return; }
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)s, (ID3D11Resource *)t);
    D3D11_MAPPED_SUBRESOURCE m;
    if (SUCCEEDED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)s, 0, D3D11_MAP_READ, 0, &m))) {
        for (int y = 0; y < H; y++) {
            printf("  ");
            for (int x = 0; x < W; x++) {
                const unsigned char *p = (const unsigned char *)m.pData + y * m.RowPitch + x * bpp;
                for (int b = bpp - 1; b >= 0; b--) printf("%02x", p[b]);
                printf(x + 1 < W ? " " : "\n");
            }
        }
        ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)s, 0);
    }
    ID3D11Texture2D_Release(s);
}

int main(void) {
    D3D_FEATURE_LEVEL want[] = { D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0 }, got;
    HRESULT hr = D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, 0, want, 2, D3D11_SDK_VERSION, &dev, &got, &ctx);
    if (FAILED(hr)) hr = D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, want, 2, D3D11_SDK_VERSION, &dev, &got, &ctx);
    if (FAILED(hr)) { printf("skip D3D11CreateDevice %#lx\n", hr); return 0; }
    if (FAILED(ID3D11DeviceContext_QueryInterface(ctx, &IID_ID3D11DeviceContext1, (void **)&ctx1))) ctx1 = NULL;

    static const UINT u[4] = { 0x12345678, 0x1ff, 0xffffffff, 3 };
    static const float fl[][4] = { { 0.5f, 1.5f, -0.25f, 1.0f }, { 1.0f / 3, 70000.0f, 1e-7f, -2.0f } };
    for (int i = 0; i < (int)(sizeof fm / sizeof fm[0]); i++) {
        printf("%s\n", fm[i].n);
        ID3D11Texture2D *t = make(fm[i].f, D3D11_BIND_UNORDERED_ACCESS | D3D11_BIND_SHADER_RESOURCE);
        if (!t) continue;
        ID3D11UnorderedAccessView *v = NULL;
        hr = ID3D11Device_CreateUnorderedAccessView(dev, (ID3D11Resource *)t, NULL, &v);
        if (FAILED(hr)) { printf("  CreateUnorderedAccessView %#lx\n", hr); ID3D11Texture2D_Release(t); continue; }
        printf(" Uint %08x %08x %08x %08x\n", u[0], u[1], u[2], u[3]);
        ID3D11DeviceContext_ClearUnorderedAccessViewUint(ctx, v, u);
        dump(t, fm[i].f, fm[i].bpp);
        for (int k = 0; k < 2; k++) {
            printf(" Float %g %g %g %g\n", fl[k][0], fl[k][1], fl[k][2], fl[k][3]);
            ID3D11DeviceContext_ClearUnorderedAccessViewFloat(ctx, v, fl[k]);
            dump(t, fm[i].f, fm[i].bpp);
        }
        float nan[4] = { NAN, -NAN, INFINITY, -INFINITY };
        printf(" Float nan -nan inf -inf\n");
        ID3D11DeviceContext_ClearUnorderedAccessViewFloat(ctx, v, nan);
        dump(t, fm[i].f, fm[i].bpp);
        if (ctx1) {
            D3D11_RECT r[2] = { { 1, 1, 3, 3 }, { 6, -1, 10, 2 } };
            float c[4] = { 0.25f, 0.75f, 1.0f, 0.5f };
            printf(" ClearView UAV rects (1,1,3,3) (6,-1,10,2)\n");
            ID3D11DeviceContext1_ClearView(ctx1, (ID3D11View *)v, c, r, 2);
            dump(t, fm[i].f, fm[i].bpp);
        }
        ID3D11UnorderedAccessView_Release(v);
        ID3D11Texture2D_Release(t);
    }

    /* ClearView of render targets */
    if (ctx1) {
        static const struct { DXGI_FORMAT f; const char *n; } rf[] = { { DXGI_FORMAT_R8G8B8A8_UNORM, "R8G8B8A8_UNORM" }, { DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, "R8G8B8A8_UNORM_SRGB" },
                                                                    { DXGI_FORMAT_B8G8R8A8_UNORM, "B8G8R8A8_UNORM" } };
        for (int i = 0; i < 3; i++) {
            ID3D11Texture2D *t = make(rf[i].f, D3D11_BIND_RENDER_TARGET);
            if (!t) continue;
            ID3D11RenderTargetView *v = NULL;
            ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)t, NULL, &v);
            float c[4] = { 0.5f, 0.25f, 1.0f, 0.75f };
            D3D11_RECT r[4] = { { 0, 0, 2, 2 }, { 5, 2, 4, 3 }, { 3, 3, 3, 4 }, { 7, 3, 100, 100 } };
            printf("%s ClearView rects (0,0,2,2) (5,2,4,3) (3,3,3,4) (7,3,100,100)\n", rf[i].n);
            ID3D11DeviceContext1_ClearView(ctx1, (ID3D11View *)v, c, r, 4);
            dump(t, rf[i].f, 4);
            printf("%s ClearView NULL 1\n", rf[i].n);
            float c2[4] = { 0.1f, 0.2f, 0.3f, 0.4f };
            ID3D11DeviceContext1_ClearView(ctx1, (ID3D11View *)v, c2, NULL, 1);
            dump(t, rf[i].f, 4);
            printf("%s ClearView rects 0\n", rf[i].n);
            ID3D11DeviceContext1_ClearView(ctx1, (ID3D11View *)v, c, r, 0);
            dump(t, rf[i].f, 4);
            ID3D11RenderTargetView_Release(v);
            ID3D11Texture2D_Release(t);
        }
    }
    return 0;
}
