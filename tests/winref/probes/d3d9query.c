/* Direct3D 9 queries: which D3DQUERYTYPEs CreateQuery makes (NULL out and not), GetDataSize,
 * GetData of a query never issued (sizes 0..12 into 0xff bytes), Issue flags 0..3 of each kind;
 * occlusion queries of pretransformed quads of known pixels (one, several, two queries open
 * at once, END without BEGIN, BEGIN twice, building, depth-tested half away, a partial Clear
 * inside, reads of 2/4/8 bytes); an event's TRUE; TIMESTAMPFREQ's value; two timestamps in a
 * row (whether the second is later); TIMESTAMPDISJOINT.
 * libs: -ld3d9
 * arch: 32
 * run: browser (sake draws through WebGPU: tests/winref/run-browser.mjs) */
#define COBJMACROS
#include "../probe.h"
#include <d3d9.h>

#define W 64
#define H 64

struct v4 { float x, y, z, rhw; DWORD c; };

static IDirect3DDevice9 *dev;

static void quad(int x0, int y0, int x1, int y1, float z) {
    struct v4 v[4] = {
        { (float)x0 - 0.5f, (float)y0 - 0.5f, z, 1, 0xff00ff00 }, { (float)x1 - 0.5f, (float)y0 - 0.5f, z, 1, 0xff00ff00 },
        { (float)x0 - 0.5f, (float)y1 - 0.5f, z, 1, 0xff00ff00 }, { (float)x1 - 0.5f, (float)y1 - 0.5f, z, 1, 0xff00ff00 },
    };
    IDirect3DDevice9_DrawPrimitiveUP(dev, D3DPT_TRIANGLESTRIP, 2, v, sizeof v[0]);
}

/* GetData(FLUSH) until it is no S_FALSE: the answer and the bytes */
static HRESULT wait(IDirect3DQuery9 *q, void *data, DWORD size) {
    HRESULT hr;
    int n = 0;
    while ((hr = IDirect3DQuery9_GetData(q, data, size, D3DGETDATA_FLUSH)) == S_FALSE && n < 1000000) n++;
    return hr;
}

static void occlusion(const char *what, IDirect3DQuery9 *q) {
    unsigned char b[12];
    memset(b, 0xff, sizeof b);
    HRESULT hr = wait(q, b, 8);
    printf("  %s: %#lx %u %u (high %02x%02x%02x%02x)\n", what, hr, b[0] | b[1] << 8 | b[2] << 16 | (unsigned)b[3] << 24, (unsigned)(b[4] | b[5] << 8), b[7], b[6], b[5], b[4]);
}

int main(void) {
    WNDCLASSA wc = { 0, DefWindowProcA, 0, 0, GetModuleHandleA(NULL), 0, 0, 0, 0, "probe" };
    RegisterClassA(&wc);
    HWND hwnd = CreateWindowA("probe", "d3d9query", WS_POPUP | WS_VISIBLE, 0, 0, W, H, 0, 0, wc.hInstance, 0);
    IDirect3D9 *d3d = Direct3DCreate9(D3D_SDK_VERSION);
    if (!d3d) { puts("skip no Direct3D 9"); return 0; }
    D3DPRESENT_PARAMETERS pp = { 0 };
    pp.BackBufferWidth = W; pp.BackBufferHeight = H; pp.BackBufferFormat = D3DFMT_X8R8G8B8; pp.BackBufferCount = 1;
    pp.SwapEffect = D3DSWAPEFFECT_DISCARD; pp.hDeviceWindow = hwnd; pp.Windowed = TRUE;
    pp.EnableAutoDepthStencil = TRUE; pp.AutoDepthStencilFormat = D3DFMT_D24S8;
    if (FAILED(IDirect3D9_CreateDevice(d3d, D3DADAPTER_DEFAULT, D3DDEVTYPE_HAL, hwnd, D3DCREATE_SOFTWARE_VERTEXPROCESSING, &pp, &dev))) { puts("skip no device"); return 0; }
    IDirect3DDevice9_SetFVF(dev, D3DFVF_XYZRHW | D3DFVF_DIFFUSE);
    IDirect3DDevice9_SetRenderState(dev, D3DRS_CULLMODE, D3DCULL_NONE);
    IDirect3DDevice9_SetRenderState(dev, D3DRS_ZENABLE, FALSE);

    puts("CreateQuery types 0..39 (NULL out, out)");
    for (int t = 0; t < 40; t++) {
        IDirect3DQuery9 *q = (IDirect3DQuery9 *)0x1234;
        HRESULT a = IDirect3DDevice9_CreateQuery(dev, (D3DQUERYTYPE)t, NULL);
        HRESULT b = IDirect3DDevice9_CreateQuery(dev, (D3DQUERYTYPE)t, &q);
        if (a != D3DERR_NOTAVAILABLE || b != D3DERR_NOTAVAILABLE) printf("  %d: %#lx %#lx%s\n", t, a, b, q == (IDirect3DQuery9 *)0x1234 ? " out untouched" : "");
        if (SUCCEEDED(b) && q && q != (IDirect3DQuery9 *)0x1234) IDirect3DQuery9_Release(q);
    }

    puts("never issued: GetDataSize, GetData of sizes 0..12 (bytes written of 16 0xff)");
    for (int t = D3DQUERYTYPE_EVENT; t <= D3DQUERYTYPE_TIMESTAMPFREQ; t++) {
        IDirect3DQuery9 *q = NULL;
        if (FAILED(IDirect3DDevice9_CreateQuery(dev, (D3DQUERYTYPE)t, &q))) { printf("  %d: none\n", t); continue; }
        printf("  %d: size %lu type %d NULL/0 %#lx:", t, IDirect3DQuery9_GetDataSize(q), IDirect3DQuery9_GetType(q), IDirect3DQuery9_GetData(q, NULL, 0, D3DGETDATA_FLUSH));
        for (DWORD n = 0; n <= 12; n++) {
            unsigned char b[16];
            memset(b, 0xff, sizeof b);
            HRESULT hr = IDirect3DQuery9_GetData(q, b, n, D3DGETDATA_FLUSH);
            int dd = 0, ff = 0;
            for (int i = 0; i < 16; i++) { dd += b[i] == 0xdd; ff += b[i] == 0xff; }
            printf(" %lu:%#lx/%d/%d", n, hr, dd, 16 - dd - ff);
        }
        putchar('\n');
        printf("    Issue 0 %#lx 1 %#lx 2 %#lx 3 %#lx\n", IDirect3DQuery9_Issue(q, 0), IDirect3DQuery9_Issue(q, D3DISSUE_END), IDirect3DQuery9_Issue(q, D3DISSUE_BEGIN), IDirect3DQuery9_Issue(q, D3DISSUE_BEGIN | D3DISSUE_END));
        IDirect3DQuery9_Release(q);
    }

    IDirect3DQuery9 *q = NULL, *q2 = NULL;
    if (FAILED(IDirect3DDevice9_CreateQuery(dev, D3DQUERYTYPE_OCCLUSION, &q)) || FAILED(IDirect3DDevice9_CreateQuery(dev, D3DQUERYTYPE_OCCLUSION, &q2))) { puts("skip no occlusion queries"); return 0; }
    puts("occlusion (count, low word, and the high dword of an 8-byte read)");
    IDirect3DDevice9_Clear(dev, 0, NULL, D3DCLEAR_TARGET | D3DCLEAR_ZBUFFER, 0, 1.0f, 0);
    IDirect3DDevice9_BeginScene(dev);
    IDirect3DQuery9_Issue(q, D3DISSUE_BEGIN);
    quad(0, 0, 10, 10, 0.5f);
    IDirect3DQuery9_Issue(q, D3DISSUE_END);
    occlusion("10x10", q);
    IDirect3DQuery9_Issue(q, D3DISSUE_BEGIN);
    quad(0, 0, 10, 10, 0.5f);
    quad(20, 20, 23, 27, 0.5f);
    quad(60, 60, 70, 70, 0.5f);
    IDirect3DQuery9_Issue(q, D3DISSUE_END);
    occlusion("10x10 + 3x7 + 4x4 inside of 10x10", q);
    IDirect3DQuery9_Issue(q, D3DISSUE_END);
    occlusion("END without BEGIN", q);
    IDirect3DQuery9_Issue(q, D3DISSUE_BEGIN);
    quad(0, 0, 8, 8, 0.5f);
    IDirect3DQuery9_Issue(q, D3DISSUE_BEGIN);
    {
        DWORD v = 0x12345678;
        printf("  building: %#lx\n", IDirect3DQuery9_GetData(q, &v, 4, D3DGETDATA_FLUSH));
    }
    quad(0, 0, 4, 2, 0.5f);
    IDirect3DQuery9_Issue(q, D3DISSUE_END);
    occlusion("BEGIN twice (4x2 after the second)", q);
    IDirect3DQuery9_Issue(q, D3DISSUE_BEGIN);
    quad(0, 0, 10, 10, 0.5f);
    IDirect3DQuery9_Issue(q2, D3DISSUE_BEGIN);
    quad(0, 0, 3, 3, 0.5f);
    IDirect3DQuery9_Issue(q, D3DISSUE_END);
    quad(0, 0, 5, 1, 0.5f);
    IDirect3DQuery9_Issue(q2, D3DISSUE_END);
    occlusion("two open: first (10x10 + 3x3)", q);
    occlusion("two open: second (3x3 + 5x1)", q2);
    {
        D3DRECT r = { 4, 4, 20, 20 };
        IDirect3DQuery9_Issue(q, D3DISSUE_BEGIN);
        IDirect3DDevice9_Clear(dev, 1, &r, D3DCLEAR_TARGET, 0xff123456, 1.0f, 0);
        quad(0, 0, 2, 2, 0.5f);
        IDirect3DQuery9_Issue(q, D3DISSUE_END);
        occlusion("a 16x16 Clear and a 2x2 quad", q);
    }
    IDirect3DDevice9_SetRenderState(dev, D3DRS_ZENABLE, TRUE);
    IDirect3DDevice9_SetRenderState(dev, D3DRS_ZFUNC, D3DCMP_LESS);
    quad(0, 0, 16, 32, 0.25f);
    IDirect3DQuery9_Issue(q, D3DISSUE_BEGIN);
    quad(0, 0, 32, 32, 0.5f);
    IDirect3DQuery9_Issue(q, D3DISSUE_END);
    occlusion("32x32 behind a 16x32 nearer half", q);
    IDirect3DDevice9_SetRenderState(dev, D3DRS_ZENABLE, FALSE);
    IDirect3DDevice9_EndScene(dev);
    {
        unsigned char b[8];
        memset(b, 0xff, sizeof b);
        HRESULT hr = IDirect3DQuery9_GetData(q, b, 2, D3DGETDATA_FLUSH);
        printf("  2 bytes: %#lx %02x %02x %02x %02x\n", hr, b[0], b[1], b[2], b[3]);
        memset(b, 0xff, sizeof b);
        hr = IDirect3DQuery9_GetData(q, b, 0, D3DGETDATA_FLUSH);
        printf("  0 bytes: %#lx %02x\n", hr, b[0]);
    }
    IDirect3DQuery9_Release(q2);
    IDirect3DQuery9_Release(q);

    puts("event, timestamps");
    IDirect3DQuery9 *ev = NULL, *ts = NULL, *fq = NULL, *dj = NULL;
    if (SUCCEEDED(IDirect3DDevice9_CreateQuery(dev, D3DQUERYTYPE_EVENT, &ev))) {
        BOOL done = 7;
        IDirect3DQuery9_Issue(ev, D3DISSUE_END);
        HRESULT hr = wait(ev, &done, sizeof done);
        printf("  event: %#lx %d\n", hr, done);
        IDirect3DQuery9_Release(ev);
    }
    if (SUCCEEDED(IDirect3DDevice9_CreateQuery(dev, D3DQUERYTYPE_TIMESTAMPFREQ, &fq))) {
        UINT64 f = 0;
        IDirect3DQuery9_Issue(fq, D3DISSUE_END);
        HRESULT hr = wait(fq, &f, sizeof f);
        printf("  frequency: %#lx nonzero %d\n", hr, f != 0);
        IDirect3DQuery9_Release(fq);
    }
    if (SUCCEEDED(IDirect3DDevice9_CreateQuery(dev, D3DQUERYTYPE_TIMESTAMP, &ts)) && SUCCEEDED(IDirect3DDevice9_CreateQuery(dev, D3DQUERYTYPE_TIMESTAMPDISJOINT, &dj))) {
        UINT64 t0 = 0, t1 = 0;
        BOOL d = 7;
        IDirect3DQuery9_Issue(dj, D3DISSUE_BEGIN);
        IDirect3DQuery9_Issue(ts, D3DISSUE_END);
        HRESULT h0 = wait(ts, &t0, sizeof t0);
        IDirect3DDevice9_BeginScene(dev);
        for (int i = 0; i < 50; i++) quad(0, 0, W, H, 0.5f);
        IDirect3DDevice9_EndScene(dev);
        IDirect3DQuery9_Issue(ts, D3DISSUE_END);
        HRESULT h1 = wait(ts, &t1, sizeof t1);
        IDirect3DQuery9_Issue(dj, D3DISSUE_END);
        HRESULT h2 = wait(dj, &d, sizeof d);
        printf("  timestamps: %#lx %#lx later %d; disjoint %#lx %d\n", h0, h1, t1 > t0, h2, d);
        IDirect3DQuery9_Release(ts);
        IDirect3DQuery9_Release(dj);
    }
    IDirect3DDevice9_Release(dev);
    IDirect3D9_Release(d3d);
    return 0;
}
