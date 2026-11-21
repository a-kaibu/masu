/* Direct3D 11 hidden counters of buffer UAVs (D3D11_BUFFER_UAV_FLAG_APPEND / COUNTER) and
 * CopyStructureCount: the count a new view copies, the initial counts CSSetUnorderedAccessViews
 * and OMSetRenderTargetsAndUnorderedAccessViews give (-1 keeps), a view without a counter, an
 * offset that is not a multiple of 4 or past the buffer's end, and what a compute shader's
 * Append / IncrementCounter / Consume / DecrementCounter make of the count and the buffer.
 * WARP or hardware on the Windows runner.
 * libs: -ld3d11 -ld3dcompiler_47
 * run: browser (sake's Direct3D 11 runs in the page: tests/winref/run-browser.mjs) */
#define COBJMACROS
#define INITGUID
#include "../probe.h"
#include <d3d11.h>
#include <d3dcompiler.h>

static ID3D11Device *dev;
static ID3D11DeviceContext *ctx;

static const char *SRC =
    "AppendStructuredBuffer<uint> a : register(u0);\n"
    "[numthreads(4, 1, 1)] void append(uint i : SV_DispatchThreadID) { a.Append(i + 100); }\n"
    "RWStructuredBuffer<uint> c : register(u0);\n"
    "[numthreads(4, 1, 1)] void inc(uint i : SV_DispatchThreadID) { c[c.IncrementCounter()] = i + 200; }\n"
    "[numthreads(2, 1, 1)] void dec(uint i : SV_DispatchThreadID) { c.DecrementCounter(); }\n"
    "ConsumeStructuredBuffer<uint> s : register(u0);\n"
    "RWStructuredBuffer<uint> o : register(u1);\n"
    "[numthreads(1, 1, 1)] void consume(uint i : SV_DispatchThreadID) { o[0] = s.Consume(); }\n";

static ID3D11ComputeShader *cs_of(const char *entry) {
    ID3DBlob *b = NULL, *err = NULL;
    HRESULT hr = D3DCompile(SRC, strlen(SRC), "uavcount", NULL, NULL, entry, "cs_5_0", 0, 0, &b, &err);
    if (err) ID3D10Blob_Release(err);
    if (FAILED(hr)) {
        printf("D3DCompile %s failed %#lx\n", entry, hr);
        return NULL;
    }
    ID3D11ComputeShader *s = NULL;
    ID3D11Device_CreateComputeShader(dev, ID3D10Blob_GetBufferPointer(b), ID3D10Blob_GetBufferSize(b), NULL, &s);
    ID3D10Blob_Release(b);
    return s;
}

static ID3D11Buffer *structured(UINT n, UINT bind) {
    static UINT init[64];
    for (int i = 0; i < 64; i++) init[i] = 0xcdcdcdcd;
    D3D11_BUFFER_DESC d = { 4 * n, D3D11_USAGE_DEFAULT, bind, 0, D3D11_RESOURCE_MISC_BUFFER_STRUCTURED, 4 };
    D3D11_SUBRESOURCE_DATA s = { init, 0, 0 };
    ID3D11Buffer *b = NULL;
    ID3D11Device_CreateBuffer(dev, &d, &s, &b);
    return b;
}

static ID3D11UnorderedAccessView *uav(ID3D11Buffer *b, UINT n, UINT flags) {
    D3D11_UNORDERED_ACCESS_VIEW_DESC d;
    memset(&d, 0, sizeof d);
    d.Format = DXGI_FORMAT_UNKNOWN;
    d.ViewDimension = D3D11_UAV_DIMENSION_BUFFER;
    d.Buffer.FirstElement = 0;
    d.Buffer.NumElements = n;
    d.Buffer.Flags = flags;
    ID3D11UnorderedAccessView *v = NULL;
    HRESULT hr = ID3D11Device_CreateUnorderedAccessView(dev, (ID3D11Resource *)b, &d, &v);
    if (FAILED(hr)) printf("  CreateUnorderedAccessView flags %u: %#lx\n", flags, hr);
    return v;
}

/* the first n words of a buffer, through a staging copy */
static void words(ID3D11Buffer *b, UINT n, UINT *out) {
    D3D11_BUFFER_DESC d;
    ID3D11Buffer_GetDesc(b, &d);
    d.Usage = D3D11_USAGE_STAGING;
    d.BindFlags = 0;
    d.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    d.MiscFlags = 0;
    d.StructureByteStride = 0;
    ID3D11Buffer *s = NULL;
    ID3D11Device_CreateBuffer(dev, &d, NULL, &s);
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)s, (ID3D11Resource *)b);
    D3D11_MAPPED_SUBRESOURCE m;
    memset(out, 0, 4 * n);
    if (SUCCEEDED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)s, 0, D3D11_MAP_READ, 0, &m))) {
        memcpy(out, m.pData, 4 * n);
        ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)s, 0);
    }
    ID3D11Buffer_Release(s);
}

static ID3D11Buffer *dst;

/* CopyStructureCount of `v` into a fresh dst (all 0xcdcdcdcd) at `at`: dst's 4 words */
static void count(const char *label, ID3D11UnorderedAccessView *v, UINT at) {
    static const UINT fill[4] = { 0xcdcdcdcd, 0xcdcdcdcd, 0xcdcdcdcd, 0xcdcdcdcd };
    ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)dst, 0, NULL, fill, 0, 0);
    ID3D11DeviceContext_CopyStructureCount(ctx, dst, at, v);
    UINT w[4];
    words(dst, 4, w);
    printf("  %s (at %u): %x %x %x %x\n", label, at, w[0], w[1], w[2], w[3]);
}

static void set_cs(ID3D11UnorderedAccessView *v, UINT c) {
    ID3D11DeviceContext_CSSetUnorderedAccessViews(ctx, 0, 1, &v, &c);
}

int main(void) {
    D3D_FEATURE_LEVEL fl;
    if (FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx)) &&
        FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx))) {
        puts("skip no device");
        return 0;
    }
    D3D11_BUFFER_DESC dd = { 16, D3D11_USAGE_DEFAULT, 0, 0, 0, 0 };
    ID3D11Device_CreateBuffer(dev, &dd, NULL, &dst);
    ID3D11Buffer *b = structured(16, D3D11_BIND_UNORDERED_ACCESS | D3D11_BIND_SHADER_RESOURCE);
    ID3D11UnorderedAccessView *app = uav(b, 16, D3D11_BUFFER_UAV_FLAG_APPEND), *cnt = uav(b, 16, D3D11_BUFFER_UAV_FLAG_COUNTER), *none = uav(b, 16, 0);
    uav(b, 16, D3D11_BUFFER_UAV_FLAG_APPEND | D3D11_BUFFER_UAV_FLAG_COUNTER);
    puts("counters of new views, then as set");
    count("new APPEND", app, 0);
    count("new COUNTER", cnt, 4);
    count("no counter", none, 0);
    set_cs(app, 5);
    count("APPEND set 5 (CS)", app, 0);
    set_cs(app, ~0u);
    count("APPEND set -1 (CS)", app, 0);
    set_cs(cnt, 0x12345678);
    count("COUNTER set 0x12345678", cnt, 8);
    set_cs(none, 9);
    count("no counter set 9", none, 0);
    count("APPEND at 2", app, 2);
    count("APPEND at 16", app, 16);
    count("APPEND at 14", app, 14);
    {
        UINT c = 3;
        ID3D11DeviceContext_OMSetRenderTargetsAndUnorderedAccessViews(ctx, D3D11_KEEP_RENDER_TARGETS_AND_DEPTH_STENCIL, NULL, NULL, 1, 1, &cnt, &c);
        count("COUNTER set 3 (OM)", cnt, 0);
        ID3D11UnorderedAccessView *nul = NULL;
        ID3D11DeviceContext_OMSetRenderTargetsAndUnorderedAccessViews(ctx, D3D11_KEEP_RENDER_TARGETS_AND_DEPTH_STENCIL, NULL, NULL, 1, 1, &nul, NULL);
    }
    {
        ID3D11UnorderedAccessView *nul = NULL;
        ID3D11DeviceContext_CSSetUnorderedAccessViews(ctx, 0, 1, &nul, NULL);
        count("COUNTER unbound", cnt, 0);
    }

    puts("compute shaders");
    ID3D11ComputeShader *s_app = cs_of("append"), *s_inc = cs_of("inc"), *s_dec = cs_of("dec"), *s_con = cs_of("consume");
    UINT w[16];
    static const UINT fill[16] = { 0xcdcdcdcd, 0xcdcdcdcd, 0xcdcdcdcd, 0xcdcdcdcd, 0xcdcdcdcd, 0xcdcdcdcd, 0xcdcdcdcd, 0xcdcdcdcd,
        0xcdcdcdcd, 0xcdcdcdcd, 0xcdcdcdcd, 0xcdcdcdcd, 0xcdcdcdcd, 0xcdcdcdcd, 0xcdcdcdcd, 0xcdcdcdcd };
    if (s_app && s_inc && s_dec && s_con) {
        ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)b, 0, NULL, fill, 0, 0);
        set_cs(app, 2);
        ID3D11DeviceContext_CSSetShader(ctx, s_app, NULL, 0);
        ID3D11DeviceContext_Dispatch(ctx, 2, 1, 1);
        count("after 8 Appends from 2", app, 0);
        words(b, 16, w);
        /* (the order of the threads' Appends is the GPU's: sorted) */
        for (int i = 0; i < 16; i++)
            for (int j = i + 1; j < 16; j++)
                if (w[j] < w[i]) { UINT t = w[i]; w[i] = w[j]; w[j] = t; }
        printf("  buffer sorted:");
        for (int i = 0; i < 16; i++) printf(" %x", w[i]);
        puts("");
        ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)b, 0, NULL, fill, 0, 0);
        set_cs(cnt, 0);
        ID3D11DeviceContext_CSSetShader(ctx, s_inc, NULL, 0);
        ID3D11DeviceContext_Dispatch(ctx, 1, 1, 1);
        count("after 4 IncrementCounter", cnt, 0);
        ID3D11DeviceContext_CSSetShader(ctx, s_dec, NULL, 0);
        set_cs(cnt, ~0u);
        ID3D11DeviceContext_Dispatch(ctx, 1, 1, 1);
        count("after 2 DecrementCounter", cnt, 0);
        words(b, 4, w);
        for (int i = 0; i < 4; i++)
            for (int j = i + 1; j < 4; j++)
                if (w[j] < w[i]) { UINT t = w[i]; w[i] = w[j]; w[j] = t; }
        printf("  buffer sorted: %x %x %x %x\n", w[0], w[1], w[2], w[3]);
        /* Consume: the element at count - 1 */
        static const UINT seq[16] = { 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25 };
        ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)b, 0, NULL, seq, 0, 0);
        ID3D11Buffer *ob = structured(1, D3D11_BIND_UNORDERED_ACCESS);
        ID3D11UnorderedAccessView *ov = uav(ob, 1, 0);
        ID3D11UnorderedAccessView *two[2] = { app, ov };
        UINT cs[2] = { 5, ~0u };
        ID3D11DeviceContext_CSSetUnorderedAccessViews(ctx, 0, 2, two, cs);
        ID3D11DeviceContext_CSSetShader(ctx, s_con, NULL, 0);
        ID3D11DeviceContext_Dispatch(ctx, 1, 1, 1);
        words(ob, 1, w);
        printf("  Consume from 5: %x\n", w[0]);
        count("after Consume", app, 0);
    }
    return 0;
}
