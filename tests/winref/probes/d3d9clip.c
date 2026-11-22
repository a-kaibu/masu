/* Direct3D 9's clipper as WARP runs it, to the last bit of the interpolated inputs: triangles
 * (random, from a fixed seed) that the near, far, left, right, top and bottom planes cut, through
 * a vs_2_0 that passes clip space through, and pretransformed (XYZRHW) ones reaching past every
 * edge of the target. A ps_2_0 shader writes t0 into an A32B32G32R32F target read back with
 * GetRenderTargetData. winref d3d9interp has two cut triangles only: the order of the cut
 * polygon's points and the rounding of the new points are what these pin down.
 * Each case prints its corners' words (x y z w and t0's 4) and then every covered pixel's 4.
 * libs: -ld3d9
 * arch: 32
 * lines: all
 * run: browser (sake draws through WebGPU: tests/winref/run-browser.mjs) */
#define COBJMACROS
#include "../probe.h"
#include <d3d9.h>
#include <string.h>

#define W 8
#define H 8

static IDirect3DDevice9 *dev;
static IDirect3DSurface9 *rt, *sys;
static unsigned seed = 0x1234567u;

static unsigned next(void) {
    seed = seed * 1664525u + 1013904223u;
    return seed >> 8;
}

/* a float in [lo, hi) on a 1/1024 grid */
static float uni(float lo, float hi) { return lo + (hi - lo) * (float)(next() % 1024) / 1024.0f; }

static unsigned bits(float f) {
    unsigned u;
    memcpy(&u, &f, 4);
    return u;
}

static void shot(void) {
    D3DLOCKED_RECT lr;
    if (FAILED(IDirect3DDevice9_GetRenderTargetData(dev, rt, sys)) || FAILED(IDirect3DSurface9_LockRect(sys, &lr, NULL, D3DLOCK_READONLY))) {
        printf("  no readback\n");
        return;
    }
    for (int y = 0; y < H; y++) {
        const DWORD *row = (const DWORD *)((const char *)lr.pBits + y * lr.Pitch);
        for (int x = 0; x < W; x++) {
            const DWORD *p = row + x * 4;
            if (p[0] | p[1] | p[2] | p[3]) printf("  %d,%d: %08lx %08lx %08lx %08lx\n", x, y, p[0], p[1], p[2], p[3]);
        }
    }
    IDirect3DSurface9_UnlockRect(sys);
}

/* ps_2_0: dcl t0; mov oC0, t0 */
static const DWORD ps_t0[] = { 0xffff0200, 0x0200001f, 0x80000000, 0xb00f0000, 0x02000001, 0x800f0800, 0xb0e40000, 0x0000ffff };
/* vs_2_0: dcl_position v0; dcl_texcoord v1; mov oPos, v0; mov oT0, v1 */
static const DWORD vs_pass[] = {
    0xfffe0200, 0x0200001f, 0x80000000, 0x900f0000, 0x0200001f, 0x80000005, 0x900f0001,
    0x02000001, 0xc00f0000, 0x90e40000, 0x02000001, 0xe00f0000, 0x90e40001, 0x0000ffff,
};

struct v8 { float p[4]; float t[4]; };

static void corners(const struct v8 *v) {
    for (int i = 0; i < 3; i++) {
        printf("  corner %d:", i);
        for (int k = 0; k < 4; k++) printf(" %08x", bits(v[i].p[k]));
        for (int k = 0; k < 4; k++) printf(" %08x", bits(v[i].t[k]));
        printf("\n");
    }
}

static void draw(const struct v8 *v, DWORD fvf) {
    IDirect3DDevice9_Clear(dev, 0, NULL, D3DCLEAR_TARGET, 0, 1, 0);
    IDirect3DDevice9_BeginScene(dev);
    IDirect3DDevice9_SetFVF(dev, fvf);
    IDirect3DDevice9_DrawPrimitiveUP(dev, D3DPT_TRIANGLELIST, 1, v, sizeof v[0]);
    IDirect3DDevice9_EndScene(dev);
    shot();
}

int main(void) {
    WNDCLASSA wc = { 0, DefWindowProcA, 0, 0, GetModuleHandleA(NULL), 0, 0, 0, 0, "probe" };
    RegisterClassA(&wc);
    HWND hwnd = CreateWindowA("probe", "d3d9clip", WS_POPUP, 0, 0, W, H, 0, 0, wc.hInstance, 0);
    IDirect3D9 *d3d = Direct3DCreate9(D3D_SDK_VERSION);
    if (!d3d) { puts("skip no Direct3D 9"); return 0; }
    D3DPRESENT_PARAMETERS pp = { 0 };
    pp.BackBufferWidth = W; pp.BackBufferHeight = H; pp.BackBufferFormat = D3DFMT_X8R8G8B8; pp.BackBufferCount = 1;
    pp.SwapEffect = D3DSWAPEFFECT_DISCARD; pp.hDeviceWindow = hwnd; pp.Windowed = TRUE;
    HRESULT hr = IDirect3D9_CreateDevice(d3d, D3DADAPTER_DEFAULT, D3DDEVTYPE_HAL, hwnd, D3DCREATE_SOFTWARE_VERTEXPROCESSING, &pp, &dev);
    if (FAILED(hr)) { printf("skip CreateDevice %#lx\n", hr); return 0; }
    hr = IDirect3DDevice9_CreateRenderTarget(dev, W, H, D3DFMT_A32B32G32R32F, D3DMULTISAMPLE_NONE, 0, FALSE, &rt, NULL);
    printf("CreateRenderTarget A32B32G32R32F %#lx\n", hr);
    if (FAILED(hr)) return 0;
    hr = IDirect3DDevice9_CreateOffscreenPlainSurface(dev, W, H, D3DFMT_A32B32G32R32F, D3DPOOL_SYSTEMMEM, &sys, NULL);
    printf("CreateOffscreenPlainSurface %#lx\n", hr);
    if (FAILED(hr)) return 0;
    IDirect3DDevice9_SetRenderTarget(dev, 0, rt);
    IDirect3DDevice9_SetRenderState(dev, D3DRS_LIGHTING, FALSE);
    IDirect3DDevice9_SetRenderState(dev, D3DRS_ZENABLE, FALSE);
    IDirect3DDevice9_SetRenderState(dev, D3DRS_CULLMODE, D3DCULL_NONE);
    IDirect3DDevice9_SetRenderState(dev, D3DRS_ALPHABLENDENABLE, FALSE);
    IDirect3DPixelShader9 *pt0;
    IDirect3DVertexShader9 *vs;
    printf("ps %#lx vs %#lx\n", IDirect3DDevice9_CreatePixelShader(dev, ps_t0, &pt0), IDirect3DDevice9_CreateVertexShader(dev, vs_pass, &vs));
    IDirect3DDevice9_SetPixelShader(dev, pt0);

    /* clip space: each corner's x, y up to 1.8 w out, z from -0.5 w to 1.3 w */
    IDirect3DDevice9_SetVertexShader(dev, vs);
    for (int c = 0; c < 48; c++) {
        struct v8 v[3];
        for (int i = 0; i < 3; i++) {
            float w = uni(0.5f, 2.0f);
            v[i].p[0] = uni(-1.8f, 1.8f) * w;
            v[i].p[1] = uni(-1.8f, 1.8f) * w;
            v[i].p[2] = uni(-0.5f, 1.3f) * w;
            v[i].p[3] = w;
            for (int k = 0; k < 4; k++) v[i].t[k] = uni(-2.0f, 2.0f);
        }
        printf("vs %d:\n", c);
        corners(v);
        draw(v, D3DFVF_XYZW | D3DFVF_TEX1 | D3DFVF_TEXCOORDSIZE4(0));
    }

    /* pretransformed: x, y from -6 to 14 (past every edge), z in 0..1, rhw 0.5 to 2 */
    IDirect3DDevice9_SetVertexShader(dev, NULL);
    for (int c = 0; c < 16; c++) {
        struct v8 v[3];
        for (int i = 0; i < 3; i++) {
            v[i].p[0] = uni(-6.0f, 14.0f);
            v[i].p[1] = uni(-6.0f, 14.0f);
            v[i].p[2] = uni(0.0f, 1.0f);
            v[i].p[3] = uni(0.5f, 2.0f);
            for (int k = 0; k < 4; k++) v[i].t[k] = uni(-2.0f, 2.0f);
        }
        printf("xyzrhw %d:\n", c);
        corners(v);
        draw(v, D3DFVF_XYZRHW | D3DFVF_TEX1 | D3DFVF_TEXCOORDSIZE4(0));
    }
    return 0;
}
