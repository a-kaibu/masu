/* Direct3D 9's interpolated pixel shader inputs to the last bit: a ps_2_0 shader writes t0 (or
 * v0) into an A32B32G32R32F target, read back with GetRenderTargetData, every covered pixel's 4
 * words printed. Whether WARP's Direct3D 9 interpolates as its Direct3D 11 does (the same
 * clipper and planes: winref d3d11interp) is what the cases ask:
 *  - pretransformed XYZRHW triangles (as games draw them) with rhw 1 and with different rhw;
 *  - vs_2_0 triangles in clip space with different w, one the left plane cuts, one the near
 *    plane cuts;
 *  - a quad of two triangles over the target, the texture coordinates games use;
 *  - the diffuse color (v0) GOURAUD.
 * Each line: x,y: the 4 words (the target cleared to 0 first).
 * libs: -ld3d9
 * arch: 32
 * lines: all
 * run: browser (sake draws through WebGPU: tests/winref/run-browser.mjs) */
#define COBJMACROS
#include "../probe.h"
#include <d3d9.h>

#define W 8
#define H 8

static IDirect3DDevice9 *dev;
static IDirect3DSurface9 *rt, *sys;

static void shot(const char *name) {
    D3DLOCKED_RECT lr;
    printf("%s:\n", name);
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

static void begin(void) {
    IDirect3DDevice9_Clear(dev, 0, NULL, D3DCLEAR_TARGET, 0, 1, 0);
    IDirect3DDevice9_BeginScene(dev);
}

/* ps_2_0: dcl t0; mov oC0, t0 */
static const DWORD ps_t0[] = { 0xffff0200, 0x0200001f, 0x80000000, 0xb00f0000, 0x02000001, 0x800f0800, 0xb0e40000, 0x0000ffff };
/* ps_2_0: dcl v0; mov oC0, v0 */
static const DWORD ps_v0[] = { 0xffff0200, 0x0200001f, 0x80000000, 0x900f0000, 0x02000001, 0x800f0800, 0x90e40000, 0x0000ffff };
/* vs_2_0: dcl_position v0; dcl_texcoord v1; mov oPos, v0; mov oT0, v1 */
static const DWORD vs_pass[] = {
    0xfffe0200, 0x0200001f, 0x80000000, 0x900f0000, 0x0200001f, 0x80000005, 0x900f0001,
    0x02000001, 0xc00f0000, 0x90e40000, 0x02000001, 0xe00f0000, 0x90e40001, 0x0000ffff,
};

struct tl { float x, y, z, rhw; DWORD c; float t[4]; };
struct cv { float x, y, z, w; float t[4]; };

#define FVF_TL (D3DFVF_XYZRHW | D3DFVF_DIFFUSE | D3DFVF_TEX1 | D3DFVF_TEXCOORDSIZE4(0))
#define FVF_CV (D3DFVF_XYZW | D3DFVF_TEX1 | D3DFVF_TEXCOORDSIZE4(0))

static void tl(const char *name, const struct tl *v, D3DPRIMITIVETYPE pt, int prims) {
    begin();
    IDirect3DDevice9_SetVertexShader(dev, NULL);
    IDirect3DDevice9_SetFVF(dev, FVF_TL);
    IDirect3DDevice9_DrawPrimitiveUP(dev, pt, prims, v, sizeof v[0]);
    IDirect3DDevice9_EndScene(dev);
    shot(name);
}

static void cv(const char *name, IDirect3DVertexShader9 *vs, const struct cv *v) {
    begin();
    IDirect3DDevice9_SetVertexShader(dev, vs);
    IDirect3DDevice9_SetFVF(dev, FVF_CV);
    IDirect3DDevice9_DrawPrimitiveUP(dev, D3DPT_TRIANGLELIST, 1, v, sizeof v[0]);
    IDirect3DDevice9_EndScene(dev);
    shot(name);
}

int main(void) {
    WNDCLASSA wc = { 0, DefWindowProcA, 0, 0, GetModuleHandleA(NULL), 0, 0, 0, 0, "probe" };
    RegisterClassA(&wc);
    HWND hwnd = CreateWindowA("probe", "d3d9interp", WS_POPUP, 0, 0, W, H, 0, 0, wc.hInstance, 0);
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

    IDirect3DPixelShader9 *pt0, *pv0;
    IDirect3DVertexShader9 *vs;
    printf("ps t0 %#lx ps v0 %#lx vs %#lx\n", IDirect3DDevice9_CreatePixelShader(dev, ps_t0, &pt0), IDirect3DDevice9_CreatePixelShader(dev, ps_v0, &pv0),
           IDirect3DDevice9_CreateVertexShader(dev, vs_pass, &vs));
    IDirect3DDevice9_SetPixelShader(dev, pt0);

    /* pretransformed, rhw 1: a triangle at fractional corners */
    struct tl a[3] = {
        { 0.3f, 0.7f, 0.25f, 1, 0xff102030, { 0.1f, 0.7f, 3.0f, 1 } },
        { 7.6f, 1.2f, 0.5f, 1, 0xff80c0e0, { 0.9f, 0.2f, -1.0f, 1 } },
        { 2.1f, 7.4f, 0.75f, 1, 0xfff00804, { 0.4f, 1.3f, 0.5f, 1 } },
    };
    tl("xyzrhw rhw 1", a, D3DPT_TRIANGLELIST, 1);
    /* the same with rhw 1, 0.5, 2: perspective-correct */
    a[1].rhw = 0.5f;
    a[2].rhw = 2;
    tl("xyzrhw rhw 1 0.5 2", a, D3DPT_TRIANGLELIST, 1);
    /* a quad over the target as games draw it: corners at -0.5, a strip */
    struct tl q[4] = {
        { -0.5f, -0.5f, 0, 1, 0xffffffff, { 0, 0, 0, 1 } },
        { W - 0.5f, -0.5f, 0, 1, 0xffffffff, { 1, 0, 0, 1 } },
        { -0.5f, H - 0.5f, 0, 1, 0xffffffff, { 0, 1, 0, 1 } },
        { W - 0.5f, H - 0.5f, 0, 1, 0xffffffff, { 1, 1, 0, 1 } },
    };
    tl("xyzrhw quad strip", q, D3DPT_TRIANGLESTRIP, 2);
    /* the diffuse color, GOURAUD */
    IDirect3DDevice9_SetPixelShader(dev, pv0);
    a[1].rhw = 1;
    a[2].rhw = 1;
    tl("xyzrhw diffuse", a, D3DPT_TRIANGLELIST, 1);
    IDirect3DDevice9_SetPixelShader(dev, pt0);

    /* clip space through vs_2_0, w 1, 2, 0.5 */
    struct cv c[3] = {
        { -0.9f, 0.8f, 0.2f, 1, { 0.1f, 0.7f, 3.0f, 1 } },
        { 1.8f, 0.6f, 1.0f, 2, { 0.9f, 0.2f, -1.0f, 1 } },
        { -0.2f, -0.45f, 0.4f, 0.5f, { 0.4f, 1.3f, 0.5f, 1 } },
    };
    cv("vs w 1 2 0.5", vs, c);
    /* the left plane cuts it */
    struct cv l[3] = {
        { -1.7f, 0.8f, 0.5f, 1, { 0.1f, 0.7f, 3.0f, 1 } },
        { 0.9f, 0.1f, 0.5f, 1, { 0.9f, 0.2f, -1.0f, 1 } },
        { -0.6f, -0.9f, 0.5f, 1, { 0.4f, 1.3f, 0.5f, 1 } },
    };
    cv("vs clipped left", vs, l);
    /* the near plane (z < 0) cuts it */
    struct cv n[3] = {
        { -0.9f, 0.8f, -0.6f, 1, { 0.1f, 0.7f, 3.0f, 1 } },
        { 1.6f, 0.4f, 1.2f, 2, { 0.9f, 0.2f, -1.0f, 1 } },
        { -0.3f, -0.4f, 0.3f, 0.5f, { 0.4f, 1.3f, 0.5f, 1 } },
    };
    cv("vs clipped near", vs, n);
    return 0;
}
