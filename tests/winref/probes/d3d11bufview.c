/* Direct3D 11 raw and structured buffer views in a compute shader: a structured SRV and UAV
 * whose FirstElement is not on a 256-byte boundary, reads past the view's end (0) and writes
 * past it (dropped), a raw view's Load/Load4 and Store2, GetDimensions of both, a row_major
 * float2x2 member, a bool member, the InterlockedAdd / InterlockedMin (signed) / InterlockedMax
 * / InterlockedCompareExchange of a raw view (the values they hand back), and what D3DReflect
 * says of the views (bind types, NumSamples, the element constant buffer).
 * WARP or hardware on the Windows runner.
 * libs: -ld3d11 -ld3dcompiler_47 -ldxguid
 * run: browser (sake's Direct3D 11 runs in the page: tests/winref/run-browser.mjs) */
#define COBJMACROS
#define INITGUID
#include "../probe.h"
#include <d3d11.h>
#include <d3dcompiler.h>
#include <d3d11shader.h>

static ID3D11Device *dev;
static ID3D11DeviceContext *ctx;

static const char *SRC =
    "struct E { uint a; float2x2 m; bool b; };\n"
    "struct R { row_major float2x2 m; };\n"
    "StructuredBuffer<E> se : register(t0);\n"
    "StructuredBuffer<R> sr : register(t1);\n"
    "ByteAddressBuffer raw : register(t2);\n"
    "RWStructuredBuffer<uint> o : register(u0);\n"
    "RWByteAddressBuffer rw : register(u1);\n"
    "[numthreads(1, 1, 1)] void main() {\n"
    "  uint n, s, rb;\n"
    "  se.GetDimensions(n, s);\n"
    "  raw.GetDimensions(rb);\n"
    "  uint on, os; o.GetDimensions(on, os);\n"
    "  o[0] = n; o[1] = s; o[2] = rb; o[3] = on;\n"
    "  o[4] = se[0].a; o[5] = se[1].a; o[6] = se[n].a; o[7] = se[n + 5].a;\n"
    "  o[8] = asuint(se[1].m[0][1]); o[9] = asuint(se[1].m[1][0]); o[10] = se[1].b ? 7 : 9; o[11] = asuint(sr[0].m[0][1]);\n"
    "  uint4 q = raw.Load4(4); o[12] = q.x; o[13] = q.w; o[14] = raw.Load(rb - 4); o[15] = raw.Load(rb);\n"
    "  rw.Store2(8, uint2(0x51, 0x52));\n"
    "  uint old; rw.InterlockedAdd(0, 5, old); o[16] = old;\n"
    "  int iold; rw.InterlockedMin(4, -3, iold); o[17] = asuint(iold);\n"
    "  rw.InterlockedMax(4, 2u, old); o[18] = old;\n"
    "  rw.InterlockedCompareExchange(16, 0x60, 0x61, old); o[19] = old;\n"
    "  rw.InterlockedCompareExchange(20, 0x99, 0x62, old); o[20] = old;\n"
    "  o[on] = 0xbad;\n"
    "  o[on + 100] = 0xbad;\n"
    "}\n";

static ID3D11Buffer *buffer(UINT bytes, UINT bind, UINT misc, UINT stride, const void *init) {
    D3D11_BUFFER_DESC d = { bytes, D3D11_USAGE_DEFAULT, bind, 0, misc, stride };
    D3D11_SUBRESOURCE_DATA s = { init, 0, 0 };
    ID3D11Buffer *b = NULL;
    HRESULT hr = ID3D11Device_CreateBuffer(dev, &d, init ? &s : NULL, &b);
    if (FAILED(hr)) printf("CreateBuffer %u bytes: %#lx\n", bytes, hr);
    return b;
}

static ID3D11ShaderResourceView *srv(ID3D11Buffer *b, UINT first, UINT n, UINT raw) {
    D3D11_SHADER_RESOURCE_VIEW_DESC d;
    memset(&d, 0, sizeof d);
    d.Format = raw ? DXGI_FORMAT_R32_TYPELESS : DXGI_FORMAT_UNKNOWN;
    d.ViewDimension = D3D11_SRV_DIMENSION_BUFFEREX;
    d.BufferEx.FirstElement = first;
    d.BufferEx.NumElements = n;
    d.BufferEx.Flags = raw ? D3D11_BUFFEREX_SRV_FLAG_RAW : 0;
    ID3D11ShaderResourceView *v = NULL;
    HRESULT hr = ID3D11Device_CreateShaderResourceView(dev, (ID3D11Resource *)b, &d, &v);
    if (FAILED(hr)) printf("CreateShaderResourceView first %u n %u raw %u: %#lx\n", first, n, raw, hr);
    return v;
}

static ID3D11UnorderedAccessView *uav(ID3D11Buffer *b, UINT first, UINT n, UINT raw) {
    D3D11_UNORDERED_ACCESS_VIEW_DESC d;
    memset(&d, 0, sizeof d);
    d.Format = raw ? DXGI_FORMAT_R32_TYPELESS : DXGI_FORMAT_UNKNOWN;
    d.ViewDimension = D3D11_UAV_DIMENSION_BUFFER;
    d.Buffer.FirstElement = first;
    d.Buffer.NumElements = n;
    d.Buffer.Flags = raw ? D3D11_BUFFER_UAV_FLAG_RAW : 0;
    ID3D11UnorderedAccessView *v = NULL;
    HRESULT hr = ID3D11Device_CreateUnorderedAccessView(dev, (ID3D11Resource *)b, &d, &v);
    if (FAILED(hr)) printf("CreateUnorderedAccessView first %u n %u raw %u: %#lx\n", first, n, raw, hr);
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

static void reflect(ID3DBlob *b) {
    ID3D11ShaderReflection *r = NULL;
    if (FAILED(D3DReflect(ID3D10Blob_GetBufferPointer(b), ID3D10Blob_GetBufferSize(b), &IID_ID3D11ShaderReflection, (void **)&r))) {
        puts("D3DReflect failed");
        return;
    }
    D3D11_SHADER_DESC sd;
    r->lpVtbl->GetDesc(r, &sd);
    printf("reflection: %u bound resources, %u constant buffers\n", sd.BoundResources, sd.ConstantBuffers);
    for (UINT i = 0; i < sd.BoundResources; i++) {
        D3D11_SHADER_INPUT_BIND_DESC d;
        r->lpVtbl->GetResourceBindingDesc(r, i, &d);
        printf("  %s: type %u point %u count %u flags %#x return %u dimension %u samples %u\n", d.Name, d.Type, d.BindPoint, d.BindCount, d.uFlags, d.ReturnType, d.Dimension, d.NumSamples);
    }
    for (UINT i = 0; i < sd.ConstantBuffers; i++) {
        ID3D11ShaderReflectionConstantBuffer *c = r->lpVtbl->GetConstantBufferByIndex(r, i);
        D3D11_SHADER_BUFFER_DESC d;
        c->lpVtbl->GetDesc(c, &d);
        printf("  cbuffer %s: type %u variables %u size %u flags %#x\n", d.Name, d.Type, d.Variables, d.Size, d.uFlags);
        for (UINT k = 0; k < d.Variables; k++) {
            ID3D11ShaderReflectionVariable *v = c->lpVtbl->GetVariableByIndex(c, k);
            D3D11_SHADER_VARIABLE_DESC vd;
            v->lpVtbl->GetDesc(v, &vd);
            D3D11_SHADER_TYPE_DESC td;
            ID3D11ShaderReflectionType *t = v->lpVtbl->GetType(v);
            t->lpVtbl->GetDesc(t, &td);
            printf("    %s: offset %u size %u class %u type %u rows %u columns %u members %u\n", vd.Name, vd.StartOffset, vd.Size, td.Class, td.Type, td.Rows, td.Columns, td.Members);
        }
    }
    r->lpVtbl->Release(r);
}

int main(void) {
    D3D_FEATURE_LEVEL fl;
    if (FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx)) &&
        FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx))) {
        puts("skip no device");
        return 0;
    }
    ID3DBlob *b = NULL, *err = NULL;
    HRESULT hr = D3DCompile(SRC, strlen(SRC), "bufview", NULL, NULL, "main", "cs_5_0", 0, 0, &b, &err);
    if (FAILED(hr)) {
        printf("D3DCompile failed %#lx: %s\n", hr, err ? (const char *)ID3D10Blob_GetBufferPointer(err) : "");
        return 0;
    }
    reflect(b);
    ID3D11ComputeShader *cs = NULL;
    ID3D11Device_CreateComputeShader(dev, ID3D10Blob_GetBufferPointer(b), ID3D10Blob_GetBufferSize(b), NULL, &cs);
    ID3D10Blob_Release(b);

    /* E is 4 + 16 + 4 = 24 bytes: 40 of them, the view from element 13 (byte 312), 6 long */
    static UINT e[40 * 6];
    for (UINT i = 0; i < 40; i++) {
        float m[4] = { 1.0f + i, 2.0f + i, 3.0f + i, 4.0f + i };
        e[i * 6] = 0x1000 + i;
        memcpy(&e[i * 6 + 1], m, 16);
        e[i * 6 + 5] = i & 1 ? 5 : 0;
    }
    ID3D11Buffer *eb = buffer(sizeof e, D3D11_BIND_SHADER_RESOURCE, D3D11_RESOURCE_MISC_BUFFER_STRUCTURED, 24, e);
    /* R is 16 bytes: 20 of them, the view from element 17 (byte 272) */
    static float r[20 * 4];
    for (UINT i = 0; i < 80; i++) r[i] = 0.5f * i;
    ID3D11Buffer *rbuf = buffer(sizeof r, D3D11_BIND_SHADER_RESOURCE, D3D11_RESOURCE_MISC_BUFFER_STRUCTURED, 16, r);
    /* raw: 128 words, the view from byte 272 (element 68), 16 words */
    static UINT raw[128];
    for (UINT i = 0; i < 128; i++) raw[i] = 0x2000 + i;
    ID3D11Buffer *rawb = buffer(sizeof raw, D3D11_BIND_SHADER_RESOURCE, D3D11_RESOURCE_MISC_BUFFER_ALLOW_RAW_VIEWS, 0, raw);
    /* o: 100 uints, the view from element 70, 24 long; the words around it 0xcdcdcdcd */
    static UINT fill[128];
    for (UINT i = 0; i < 128; i++) fill[i] = 0xcdcdcdcd;
    ID3D11Buffer *ob = buffer(400, D3D11_BIND_UNORDERED_ACCESS, D3D11_RESOURCE_MISC_BUFFER_STRUCTURED, 4, fill);
    /* rw: 96 words, the view from byte 336 (element 84), 8 words */
    static UINT rwi[96];
    for (UINT i = 0; i < 96; i++) rwi[i] = 0x3000 + i;
    rwi[84] = 10;
    rwi[85] = 1;
    rwi[88] = 0x60;
    rwi[89] = 0x70;
    ID3D11Buffer *rwb = buffer(sizeof rwi, D3D11_BIND_UNORDERED_ACCESS, D3D11_RESOURCE_MISC_BUFFER_ALLOW_RAW_VIEWS, 0, rwi);

    ID3D11ShaderResourceView *srvs[3] = { srv(eb, 13, 6, 0), srv(rbuf, 17, 3, 0), srv(rawb, 68, 16, 1) };
    ID3D11UnorderedAccessView *uavs[2] = { uav(ob, 70, 24, 0), uav(rwb, 84, 8, 1) };
    ID3D11DeviceContext_CSSetShaderResources(ctx, 0, 3, srvs);
    ID3D11DeviceContext_CSSetUnorderedAccessViews(ctx, 0, 2, uavs, NULL);
    ID3D11DeviceContext_CSSetShader(ctx, cs, NULL, 0);
    ID3D11DeviceContext_Dispatch(ctx, 1, 1, 1);

    UINT w[128];
    words(ob, 100, w);
    puts("o (the view: 70..93)");
    for (UINT i = 66; i < 100; i++) printf("  o[%u] %x\n", i, w[i]);
    words(rwb, 96, w);
    puts("rw (the view: 84..91)");
    for (UINT i = 82; i < 94; i++) printf("  rw[%u] %x\n", i, w[i]);
    return 0;
}
