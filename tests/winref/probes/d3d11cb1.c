/* Direct3D 11.1 constant buffer ranges (ID3D11DeviceContext1::*SetConstantBuffers1): what a
 * pixel shader reads through a range (first constant, count), past the range and past the
 * buffer's end, ranges that are not multiples of 16, a NULL first/count array; what
 * Get*ConstantBuffers1 gives back for each stage after Set1 and after the plain Set; the
 * D3D11_OPTIONS bits; UpdateSubresource1 and CopySubresourceRegion1 with their flags.
 * WARP or hardware on the Windows runner.
 * libs: -ld3d11 -ld3dcompiler_47
 * run: browser (sake's Direct3D 11 runs in the page: tests/winref/run-browser.mjs) */
#define COBJMACROS
#define INITGUID
#include "../probe.h"
#include <d3d11_1.h>
#include <d3dcompiler.h>

#define N 32 /* pixels read: constants 0..31 of the bound range */
#define CONSTANTS 64

static ID3D11Device *dev;
static ID3D11DeviceContext *ctx;
static ID3D11DeviceContext1 *ctx1;
static ID3D11Texture2D *rt, *stage;
static ID3D11Buffer *cb, *cb2;

static const char *SRC =
    "cbuffer C : register(b0) { uint4 v[32]; };\n"
    "float4 vs(uint id : SV_VertexID) : SV_Position {\n"
    "    float2 p = float2((id << 1) & 2, id & 2);\n"
    "    return float4(p * float2(2, -2) + float2(-1, 1), 0, 1);\n"
    "}\n"
    "uint4 ps(float4 p : SV_Position) : SV_Target { return v[(uint)p.x]; }\n";

static ID3DBlob *compile(const char *entry, const char *target) {
    ID3DBlob *b = NULL, *err = NULL;
    HRESULT hr = D3DCompile(SRC, strlen(SRC), "cb1", NULL, NULL, entry, target, 0, 0, &b, &err);
    if (err) ID3D10Blob_Release(err);
    if (FAILED(hr)) printf("D3DCompile %s failed %#lx\n", entry, hr);
    return b;
}

/* what the pixel shader read, one line: the buffer's constant each pixel saw ("-" zeros; the
 * other three words follow from the first, checked: "!" marks one that does not) */
static void draw(const char *name) {
    float clear[4] = { 0, 0, 0, 0 };
    ID3D11RenderTargetView *rtv = NULL;
    ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)rt, NULL, &rtv);
    ID3D11DeviceContext_ClearRenderTargetView(ctx, rtv, clear);
    ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &rtv, NULL);
    ID3D11DeviceContext_Draw(ctx, 3, 0);
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)stage, (ID3D11Resource *)rt);
    D3D11_MAPPED_SUBRESOURCE m;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)stage, 0, D3D11_MAP_READ, 0, &m))) {
        printf("%s: Map failed\n", name);
        ID3D11RenderTargetView_Release(rtv);
        return;
    }
    printf("%s:", name);
    const UINT *p = (const UINT *)m.pData;
    for (int x = 0; x < N; x++) {
        const UINT *c = p + x * 4;
        int same = c[0] == 0 ? c[1] == 0 && c[2] == 0 && c[3] == 0 : c[1] == c[0] + 1000 && c[2] == c[0] + 2000 && c[3] == c[0] + 3000;
        if (c[0] == 0) printf(" -%s", same ? "" : "!");
        else printf(" %u%s", c[0] - 1, same ? "" : "!");
    }
    printf("\n");
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)stage, 0);
    ID3D11RenderTargetView_Release(rtv);
}

static const char *STAGE[6] = { "VS", "HS", "DS", "GS", "PS", "CS" };

static void set1(int s, UINT slot, ID3D11Buffer *b, const UINT *first, const UINT *num) {
    ID3D11Buffer *bs[1] = { b };
    switch (s) {
    case 0: ID3D11DeviceContext1_VSSetConstantBuffers1(ctx1, slot, 1, bs, first, num); break;
    case 1: ID3D11DeviceContext1_HSSetConstantBuffers1(ctx1, slot, 1, bs, first, num); break;
    case 2: ID3D11DeviceContext1_DSSetConstantBuffers1(ctx1, slot, 1, bs, first, num); break;
    case 3: ID3D11DeviceContext1_GSSetConstantBuffers1(ctx1, slot, 1, bs, first, num); break;
    case 4: ID3D11DeviceContext1_PSSetConstantBuffers1(ctx1, slot, 1, bs, first, num); break;
    default: ID3D11DeviceContext1_CSSetConstantBuffers1(ctx1, slot, 1, bs, first, num); break;
    }
}

static void get1(int s, UINT slot, UINT n, ID3D11Buffer **bs, UINT *first, UINT *num) {
    switch (s) {
    case 0: ID3D11DeviceContext1_VSGetConstantBuffers1(ctx1, slot, n, bs, first, num); break;
    case 1: ID3D11DeviceContext1_HSGetConstantBuffers1(ctx1, slot, n, bs, first, num); break;
    case 2: ID3D11DeviceContext1_DSGetConstantBuffers1(ctx1, slot, n, bs, first, num); break;
    case 3: ID3D11DeviceContext1_GSGetConstantBuffers1(ctx1, slot, n, bs, first, num); break;
    case 4: ID3D11DeviceContext1_PSGetConstantBuffers1(ctx1, slot, n, bs, first, num); break;
    default: ID3D11DeviceContext1_CSGetConstantBuffers1(ctx1, slot, n, bs, first, num); break;
    }
}

/* slots 2..3 of stage s: the buffer (cb, cb2, 0 or x), its first constant and count */
static void show(const char *name, int s) {
    ID3D11Buffer *bs[2] = { (ID3D11Buffer *)1, (ID3D11Buffer *)1 };
    UINT first[2] = { 0xaaaa, 0xaaaa }, num[2] = { 0xbbbb, 0xbbbb };
    get1(s, 2, 2, bs, first, num);
    printf("%s %s:", name, STAGE[s]);
    for (int i = 0; i < 2; i++) {
        printf(" %s %#x %#x", bs[i] == cb ? "cb" : bs[i] == cb2 ? "cb2" : bs[i] ? "x" : "0", first[i], num[i]);
        if (bs[i]) ID3D11Buffer_Release(bs[i]);
    }
    printf("\n");
}

static void range(const char *name, UINT first, UINT num) {
    set1(4, 0, cb, &first, &num);
    draw(name);
}

int main(void) {
    static const D3D_FEATURE_LEVEL want[2] = { D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0 };
    D3D_FEATURE_LEVEL got = 0;
    HRESULT hr = D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, 0, want, 2, D3D11_SDK_VERSION, &dev, &got, &ctx);
    if (FAILED(hr)) hr = D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, want, 2, D3D11_SDK_VERSION, &dev, &got, &ctx);
    if (FAILED(hr)) { printf("skip D3D11CreateDevice %#lx\n", hr); return 0; }
    if (FAILED(ID3D11DeviceContext_QueryInterface(ctx, &IID_ID3D11DeviceContext1, (void **)&ctx1))) { printf("skip ID3D11DeviceContext1\n"); return 0; }

    D3D11_FEATURE_DATA_D3D11_OPTIONS o;
    memset(&o, 0, sizeof o);
    hr = ID3D11Device_CheckFeatureSupport(dev, D3D11_FEATURE_D3D11_OPTIONS, &o, sizeof o);
    printf("D3D11_OPTIONS %#lx size %u: LogicOp %d UAVOnlyForcedSampleCount %d DiscardAPIs %d FlagsForUpdateAndCopy %d ClearView %d CopyWithOverlap %d "
           "ConstantBufferPartialUpdate %d ConstantBufferOffsetting %d MapNoOverwriteDynamicCB %d MapNoOverwriteDynamicSRV %d\n", hr, (unsigned)sizeof o,
           o.OutputMergerLogicOp, o.UAVOnlyRenderingForcedSampleCount, o.DiscardAPIsSeenByDriver, o.FlagsForUpdateAndCopySeenByDriver, o.ClearView,
           o.CopyWithOverlap, o.ConstantBufferPartialUpdate, o.ConstantBufferOffsetting, o.MapNoOverwriteOnDynamicConstantBuffer, o.MapNoOverwriteOnDynamicBufferSRV);

    D3D11_TEXTURE2D_DESC td = { N, 1, 1, 1, DXGI_FORMAT_R32G32B32A32_UINT, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET, 0, 0 };
    ID3D11Device_CreateTexture2D(dev, &td, NULL, &rt);
    td.Usage = D3D11_USAGE_STAGING; td.BindFlags = 0; td.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    ID3D11Device_CreateTexture2D(dev, &td, NULL, &stage);

    /* constant k holds (k + 1, k + 1001, k + 2001, k + 3001): 0 is what nothing bound reads */
    static UINT data[CONSTANTS * 4];
    for (int k = 0; k < CONSTANTS; k++)
        for (int c = 0; c < 4; c++) data[k * 4 + c] = k + 1 + c * 1000;
    D3D11_BUFFER_DESC bd = { sizeof data, D3D11_USAGE_DEFAULT, D3D11_BIND_CONSTANT_BUFFER, 0, 0, 0 };
    D3D11_SUBRESOURCE_DATA sd = { data, 0, 0 };
    if (FAILED(ID3D11Device_CreateBuffer(dev, &bd, &sd, &cb))) { printf("CreateBuffer failed\n"); return 0; }
    ID3D11Device_CreateBuffer(dev, &bd, &sd, &cb2);

    ID3DBlob *vb = compile("vs", "vs_4_0"), *pb = compile("ps", "ps_4_0");
    if (!vb || !pb) return 0;
    ID3D11VertexShader *vs = NULL;
    ID3D11PixelShader *ps = NULL;
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(vb), ID3D10Blob_GetBufferSize(vb), NULL, &vs);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(pb), ID3D10Blob_GetBufferSize(pb), NULL, &ps);
    ID3D11DeviceContext_VSSetShader(ctx, vs, NULL, 0);
    ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
    ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    D3D11_VIEWPORT vp = { 0, 0, N, 1, 0, 1 };
    ID3D11DeviceContext_RSSetViewports(ctx, 1, &vp);

    /* what the shader reads */
    ID3D11DeviceContext_PSSetConstantBuffers(ctx, 0, 1, &cb);
    draw("set");
    range("first0-num4096", 0, 4096);
    range("first16-num16", 16, 16);
    range("first32-num16", 32, 16);
    range("first48-num32", 48, 32);
    range("first64-num16", 64, 16);
    range("first0-num16", 0, 16);
    range("first8-num16", 8, 16);
    range("first16-num8", 16, 8);
    range("first16-num0", 16, 0);
    range("first16-num4112", 16, 4112);
    {
        ID3D11Buffer *bs[1] = { cb };
        ID3D11DeviceContext1_PSSetConstantBuffers1(ctx1, 0, 1, bs, NULL, NULL);
        draw("set1-null-ranges");
        UINT f = 16;
        ID3D11DeviceContext1_PSSetConstantBuffers1(ctx1, 0, 1, bs, &f, NULL);
        draw("set1-null-num");
        UINT n = 16;
        ID3D11DeviceContext1_PSSetConstantBuffers1(ctx1, 0, 1, bs, NULL, &n);
        draw("set1-null-first");
    }
    /* a range, then the plain Set of the same slot: the whole buffer again */
    range("again-first16-num16", 16, 16);
    ID3D11DeviceContext_PSSetConstantBuffers(ctx, 0, 1, &cb);
    draw("plain-after-range");

    /* the bytes change under a range: UpdateSubresource1 with and without its flags */
    {
        UINT f = 16, n = 16;
        set1(4, 0, cb, &f, &n);
        UINT k20[4] = { 501, 1501, 2501, 3501 };
        D3D11_BOX box = { 20 * 16, 0, 0, 21 * 16, 1, 1 };
        ID3D11DeviceContext1_UpdateSubresource1(ctx1, (ID3D11Resource *)cb, 0, &box, k20, 0, 0, D3D11_COPY_NO_OVERWRITE);
        draw("update1-no-overwrite");
        UINT k21[4] = { 601, 1601, 2601, 3601 };
        box.left = 21 * 16; box.right = 22 * 16;
        ID3D11DeviceContext1_UpdateSubresource1(ctx1, (ID3D11Resource *)cb, 0, &box, k21, 0, 0, D3D11_COPY_DISCARD);
        draw("update1-discard");
        ID3D11DeviceContext1_UpdateSubresource1(ctx1, (ID3D11Resource *)cb, 0, NULL, data, 0, 0, 0);
        draw("update1-whole");
        D3D11_BOX src = { 40 * 16, 0, 0, 41 * 16, 1, 1 };
        ID3D11DeviceContext1_CopySubresourceRegion1(ctx1, (ID3D11Resource *)cb, 0, 17 * 16, 0, 0, (ID3D11Resource *)cb2, 0, &src, D3D11_COPY_NO_OVERWRITE);
        draw("copy1-no-overwrite");
        src.left = 41 * 16; src.right = 42 * 16;
        ID3D11DeviceContext1_CopySubresourceRegion1(ctx1, (ID3D11Resource *)cb, 0, 18 * 16, 0, 0, (ID3D11Resource *)cb2, 0, &src, D3D11_COPY_DISCARD);
        draw("copy1-discard");
        ID3D11DeviceContext1_UpdateSubresource1(ctx1, (ID3D11Resource *)cb, 0, NULL, data, 0, 0, 0);
    }

    /* what Get*ConstantBuffers1 gives back, each stage */
    for (int s = 0; s < 6; s++) {
        UINT f[1] = { 16 * (UINT)(s + 1) }, n[1] = { 16 * (UINT)(s + 2) };
        set1(s, 2, cb, f, n);
        set1(s, 3, cb2, NULL, NULL);
        show("set1", s);
    }
    {
        ID3D11Buffer *bs[2] = { cb2, cb };
        ID3D11DeviceContext_VSSetConstantBuffers(ctx, 2, 2, bs);
        ID3D11DeviceContext_CSSetConstantBuffers(ctx, 2, 2, bs);
        show("plain", 0);
        show("plain", 5);
        ID3D11DeviceContext_VSSetConstantBuffers(ctx, 2, 2, NULL);
        show("plain-null", 0);
        /* the arrays one at a time, and past the slots */
        UINT f[2] = { 0xcccc, 0xcccc }, n[2] = { 0xdddd, 0xdddd };
        ID3D11DeviceContext1_CSGetConstantBuffers1(ctx1, 2, 2, NULL, f, NULL);
        printf("get1 firsts only: %#x %#x\n", f[0], f[1]);
        ID3D11DeviceContext1_CSGetConstantBuffers1(ctx1, 2, 2, NULL, NULL, n);
        printf("get1 counts only: %#x %#x\n", n[0], n[1]);
        f[0] = f[1] = n[0] = n[1] = 0xeeee;
        ID3D11DeviceContext1_CSGetConstantBuffers1(ctx1, 13, 2, NULL, f, n);
        printf("get1 past the slots: %#x %#x %#x %#x\n", f[0], f[1], n[0], n[1]);
        ID3D11DeviceContext_ClearState(ctx);
        show("after ClearState", 5);
    }
    return 0;
}
