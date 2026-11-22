/* Direct3D 11.1 logic operations (CreateBlendState1's LogicOpEnable / LogicOp) as the output
 * merger applies them:
 *  - each of the 16 D3D11_LOGIC_OPs on R8G8B8A8_UINT, R16G16_UINT, R32_UINT and
 *    R10G10B10A2_UINT targets: a pixel shader's constant source against a known destination;
 *  - a source wider than the channel (COPY, XOR, and without a logic op);
 *  - two triangles of one draw over the same pixels (XOR twice, OR then AND...: the second
 *    reads what the first wrote), a triangle strip's odd triangle, instances;
 *  - RenderTargetWriteMask with a logic op; a logic op on the second target only
 *    (IndependentBlendEnable), the first written plainly;
 *  - a logic op on an R8G8B8A8_UNORM target.
 * WARP or hardware on the Windows runner.
 * libs: -ld3d11 -ld3dcompiler_47
 * run: browser (sake's Direct3D 11 runs in the page: tests/winref/run-browser.mjs) */
#define COBJMACROS
#define INITGUID
#include "../probe.h"
#include <d3d11_1.h>
#include <d3dcompiler.h>

#define W 4
#define H 4

static ID3D11Device *dev;
static ID3D11Device1 *dev1;
static ID3D11DeviceContext *ctx;

static const char *SRC =
    "cbuffer C : register(b0) { uint4 src; uint4 src1; float4 srcf; };\n"
    "float4 vs_full(uint id : SV_VertexID) : SV_Position { return float4((id >> 1) * 4.0 - 1.0, (id & 1) * 4.0 - 1.0, 0.5, 1); }\n"
    /* two full-target triangles per instance, the second with the first's corners */
    "float4 vs_twice(uint id : SV_VertexID) : SV_Position { uint k = id % 3; return float4((k >> 1) * 4.0 - 1.0, (k & 1) * 4.0 - 1.0, 0.5, 1); }\n"
    /* a strip of 4 over the whole target: triangles 0 1 2 and 1 2 3 (the odd one wound the other way) */
    "float4 vs_strip(uint id : SV_VertexID) : SV_Position { return float4((id >> 1) * 2.0 - 1.0, (id & 1) * 2.0 - 1.0, 0.5, 1); }\n"
    "uint4 ps(float4 p : SV_Position) : SV_Target { return src; }\n"
    "struct O2 { uint4 a : SV_Target0; uint4 b : SV_Target1; };\n"
    "O2 ps2(float4 p : SV_Position) { O2 o; o.a = src; o.b = src1; return o; }\n"
    "float4 psf(float4 p : SV_Position) : SV_Target { return srcf; }\n";

static ID3DBlob *compile(const char *entry, const char *target) {
    ID3DBlob *b = NULL, *err = NULL;
    HRESULT hr = D3DCompile(SRC, strlen(SRC), "logic", NULL, NULL, entry, target, 0, 0, &b, &err);
    if (FAILED(hr)) printf("D3DCompile %s failed %#lx %s\n", entry, hr, err ? (char *)ID3D10Blob_GetBufferPointer(err) : "");
    if (err) ID3D10Blob_Release(err);
    return b;
}

static ID3D11Buffer *cb;

static void constants(const UINT s[4], const UINT s1[4], const float f[4]) {
    UINT d[12] = { 0 };
    memcpy(d, s, 16);
    if (s1) memcpy(d + 4, s1, 16);
    if (f) memcpy(d + 8, f, 16);
    ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)cb, 0, NULL, d, 0, 0);
}

struct target {
    ID3D11Texture2D *t, *staging;
    ID3D11RenderTargetView *v;
    DXGI_FORMAT f;
    UINT bpp;
};

static int target(struct target *t, DXGI_FORMAT f, UINT bpp) {
    D3D11_TEXTURE2D_DESC td = { W, H, 1, 1, f, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET, 0, 0 };
    t->f = f;
    t->bpp = bpp;
    HRESULT hr = ID3D11Device_CreateTexture2D(dev, &td, NULL, &t->t);
    if (FAILED(hr)) { printf("CreateTexture2D %u failed %#lx\n", f, hr); return 0; }
    td.Usage = D3D11_USAGE_STAGING;
    td.BindFlags = 0;
    td.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    ID3D11Device_CreateTexture2D(dev, &td, NULL, &t->staging);
    ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)t->t, NULL, &t->v);
    return 1;
}

/* every texel of the target: `px` (bpp bytes, repeated) */
static void fill(struct target *t, const void *px) {
    unsigned char row[W * 8];
    for (int x = 0; x < W; x++) memcpy(row + x * t->bpp, px, t->bpp);
    unsigned char all[W * 8 * H];
    for (int y = 0; y < H; y++) memcpy(all + y * W * t->bpp, row, W * t->bpp);
    ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)t->t, 0, NULL, all, W * t->bpp, 0);
}

/* texel (x, y) as hex words */
static void show(struct target *t, int x, int y) {
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)t->staging, (ID3D11Resource *)t->t);
    D3D11_MAPPED_SUBRESOURCE m;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)t->staging, 0, D3D11_MAP_READ, 0, &m))) { printf(" map failed"); return; }
    const unsigned char *p = (const unsigned char *)m.pData + y * m.RowPitch + x * t->bpp;
    for (UINT k = 0; k < t->bpp; k += 4) {
        UINT v;
        memcpy(&v, p + k, 4);
        printf(" %08x", v);
    }
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)t->staging, 0);
}

/* the same texel on every pixel, or the first that differs */
static void uniform(struct target *t) {
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)t->staging, (ID3D11Resource *)t->t);
    D3D11_MAPPED_SUBRESOURCE m;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)t->staging, 0, D3D11_MAP_READ, 0, &m))) return;
    const unsigned char *p0 = m.pData;
    int bad = -1;
    for (int y = 0; y < H && bad < 0; y++)
        for (int x = 0; x < W; x++)
            if (memcmp((const unsigned char *)m.pData + y * m.RowPitch + x * t->bpp, p0, t->bpp)) { bad = y * W + x; break; }
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)t->staging, 0);
    if (bad >= 0) printf(" (pixel %d differs)", bad);
}

static ID3D11BlendState *blend(BOOL independent, UINT rt, BOOL logic, D3D11_LOGIC_OP op, UINT mask) {
    D3D11_BLEND_DESC1 b;
    memset(&b, 0, sizeof b);
    b.IndependentBlendEnable = independent;
    for (int i = 0; i < 8; i++) {
        b.RenderTarget[i].SrcBlend = b.RenderTarget[i].SrcBlendAlpha = D3D11_BLEND_ONE;
        b.RenderTarget[i].DestBlend = b.RenderTarget[i].DestBlendAlpha = D3D11_BLEND_ZERO;
        b.RenderTarget[i].BlendOp = b.RenderTarget[i].BlendOpAlpha = D3D11_BLEND_OP_ADD;
        b.RenderTarget[i].LogicOp = D3D11_LOGIC_OP_NOOP;
        b.RenderTarget[i].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL;
    }
    b.RenderTarget[rt].LogicOpEnable = logic;
    b.RenderTarget[rt].LogicOp = op;
    b.RenderTarget[rt].RenderTargetWriteMask = (UINT8)mask;
    ID3D11BlendState1 *s = NULL;
    HRESULT hr = ID3D11Device1_CreateBlendState1(dev1, &b, &s);
    if (FAILED(hr)) printf(" CreateBlendState1 failed %#lx", hr);
    return (ID3D11BlendState *)s;
}

static const char *OPS[16] = { "clear", "set", "copy", "copy_inverted", "noop", "invert", "and", "nand", "or", "nor", "xor", "equiv", "and_reverse", "and_inverted", "or_reverse", "or_inverted" };

int main(void) {
    D3D_FEATURE_LEVEL fl;
    if (FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx)) &&
        FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx))) {
        puts("skip no device");
        return 0;
    }
    if (fl < D3D_FEATURE_LEVEL_11_0) { puts("skip feature level below 11_0"); return 0; }
    if (FAILED(ID3D11Device_QueryInterface(dev, &IID_ID3D11Device1, (void **)&dev1))) { puts("skip no ID3D11Device1"); return 0; }
    D3D11_FEATURE_DATA_D3D11_OPTIONS o;
    memset(&o, 0, sizeof o);
    ID3D11Device_CheckFeatureSupport(dev, D3D11_FEATURE_D3D11_OPTIONS, &o, sizeof o);
    printf("options logic_op %d\n", o.OutputMergerLogicOp);
    if (!o.OutputMergerLogicOp) return 0;

    static const char *names[] = { "vs_full", "vs_twice", "vs_strip", "ps", "ps2", "psf" };
    static const char *kinds[] = { "vs_4_0", "vs_4_0", "vs_4_0", "ps_4_0", "ps_4_0", "ps_4_0" };
    ID3DBlob *bl[6];
    for (int i = 0; i < 6; i++)
        if (!(bl[i] = compile(names[i], kinds[i]))) return 0;
    ID3D11VertexShader *vs_full, *vs_twice, *vs_strip;
    ID3D11PixelShader *ps, *ps2, *psf;
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(bl[0]), ID3D10Blob_GetBufferSize(bl[0]), NULL, &vs_full);
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(bl[1]), ID3D10Blob_GetBufferSize(bl[1]), NULL, &vs_twice);
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(bl[2]), ID3D10Blob_GetBufferSize(bl[2]), NULL, &vs_strip);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[3]), ID3D10Blob_GetBufferSize(bl[3]), NULL, &ps);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[4]), ID3D10Blob_GetBufferSize(bl[4]), NULL, &ps2);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[5]), ID3D10Blob_GetBufferSize(bl[5]), NULL, &psf);
    {
        D3D11_BUFFER_DESC bd = { 48, D3D11_USAGE_DEFAULT, D3D11_BIND_CONSTANT_BUFFER, 0, 0, 0 };
        ID3D11Device_CreateBuffer(dev, &bd, NULL, &cb);
    }
    ID3D11DeviceContext_PSSetConstantBuffers(ctx, 0, 1, &cb);
    D3D11_VIEWPORT vp = { 0, 0, W, H, 0, 1 };
    ID3D11DeviceContext_RSSetViewports(ctx, 1, &vp);
    ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    float zero[4] = { 0, 0, 0, 0 };

    struct target t[4];
    static const DXGI_FORMAT fmts[4] = { DXGI_FORMAT_R8G8B8A8_UINT, DXGI_FORMAT_R16G16_UINT, DXGI_FORMAT_R32_UINT, DXGI_FORMAT_R10G10B10A2_UINT };
    static const char *fnames[4] = { "rgba8_uint", "rg16_uint", "r32_uint", "rgb10a2_uint" };
    /* each format's destination texel, and the source the shader gives (within the channels) */
    static const UINT dst[4] = { 0xc3a55a3c, 0xf00f0ff0, 0x12345678, 0x9abcdef0 };
    static const UINT srcs[4][4] = { { 0x96, 0x0f, 0xf0, 0x33 }, { 0x6996, 0x1ee1, 0, 0 }, { 0xdeadbeef, 0, 0, 0 }, { 0x2a5, 0x15a, 0x3c3, 1 } };
    for (int f = 0; f < 4; f++) {
        if (!target(&t[f], fmts[f], 4)) return 0;
        printf("%s: dst %08x src %x %x %x %x\n", fnames[f], dst[f], srcs[f][0], srcs[f][1], srcs[f][2], srcs[f][3]);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &t[f].v, NULL);
        ID3D11DeviceContext_VSSetShader(ctx, vs_full, NULL, 0);
        ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
        constants(srcs[f], NULL, NULL);
        for (int op = 0; op < 16; op++) {
            fill(&t[f], &dst[f]);
            ID3D11BlendState *b = blend(FALSE, 0, TRUE, (D3D11_LOGIC_OP)op, 0xf);
            ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            printf("  %s:", OPS[op]);
            show(&t[f], 1, 2);
            uniform(&t[f]);
            puts("");
            if (b) ID3D11BlendState_Release(b);
        }
    }

    puts("a source wider than the channels (rgba8_uint, rgb10a2_uint)");
    {
        static const UINT wide[4] = { 0x12345601, 0x1ff, 0x100, 0xffffffff };
        constants(wide, NULL, NULL);
        for (int k = 0; k < 2; k++) {
            int f = k ? 3 : 0;
            ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &t[f].v, NULL);
            for (int m = 0; m < 3; m++) {
                fill(&t[f], &dst[f]);
                ID3D11BlendState *b = blend(FALSE, 0, m > 0, m == 2 ? D3D11_LOGIC_OP_XOR : D3D11_LOGIC_OP_COPY, 0xf);
                ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
                ID3D11DeviceContext_Draw(ctx, 3, 0);
                printf("  %s %s:", fnames[f], m == 0 ? "no logic op" : m == 1 ? "copy" : "xor");
                show(&t[f], 0, 0);
                puts("");
                if (b) ID3D11BlendState_Release(b);
            }
        }
    }

    puts("two triangles of one draw over the same pixels (r32_uint)");
    {
        static const UINT s[4] = { 0x0ff000ff, 0, 0, 0 };
        constants(s, NULL, NULL);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &t[2].v, NULL);
        ID3D11DeviceContext_VSSetShader(ctx, vs_twice, NULL, 0);
        static const D3D11_LOGIC_OP ops[] = { D3D11_LOGIC_OP_XOR, D3D11_LOGIC_OP_INVERT, D3D11_LOGIC_OP_EQUIV, D3D11_LOGIC_OP_NAND, D3D11_LOGIC_OP_AND_REVERSE, D3D11_LOGIC_OP_OR_INVERTED };
        static const char *on[] = { "xor", "invert", "equiv", "nand", "and_reverse", "or_inverted" };
        for (int i = 0; i < 6; i++) {
            for (int inst = 1; inst <= 2; inst++) {
                fill(&t[2], &dst[2]);
                ID3D11BlendState *b = blend(FALSE, 0, TRUE, ops[i], 0xf);
                ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
                ID3D11DeviceContext_DrawInstanced(ctx, 6, inst, 0, 0);
                printf("  %s, %d instance%s:", on[i], inst, inst > 1 ? "s" : "");
                show(&t[2], 2, 1);
                uniform(&t[2]);
                puts("");
                if (b) ID3D11BlendState_Release(b);
            }
        }
        /* a strip of two triangles: each pixel under one of them, once */
        ID3D11DeviceContext_VSSetShader(ctx, vs_strip, NULL, 0);
        ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        fill(&t[2], &dst[2]);
        ID3D11BlendState *b = blend(FALSE, 0, TRUE, D3D11_LOGIC_OP_XOR, 0xf);
        ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
        ID3D11DeviceContext_Draw(ctx, 4, 0);
        printf("  xor, a strip of 2:");
        for (int i = 0; i < W * H; i++) show(&t[2], i % W, i / W);
        puts("");
        if (b) ID3D11BlendState_Release(b);
        ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        ID3D11DeviceContext_VSSetShader(ctx, vs_full, NULL, 0);
    }

    puts("write masks with xor (rgba8_uint)");
    {
        constants(srcs[0], NULL, NULL);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &t[0].v, NULL);
        static const UINT masks[] = { 0, 1, 2, 4, 8, 5, 0xa, 0xe };
        for (int i = 0; i < 8; i++) {
            fill(&t[0], &dst[0]);
            ID3D11BlendState *b = blend(FALSE, 0, TRUE, D3D11_LOGIC_OP_XOR, masks[i]);
            ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            printf("  mask %x:", masks[i]);
            show(&t[0], 3, 3);
            puts("");
            if (b) ID3D11BlendState_Release(b);
        }
    }

    puts("a logic op on the second target only (rgba8_uint, r32_uint)");
    {
        constants(srcs[0], srcs[2], NULL);
        ID3D11RenderTargetView *two[2] = { t[0].v, t[2].v };
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 2, two, NULL);
        ID3D11DeviceContext_PSSetShader(ctx, ps2, NULL, 0);
        fill(&t[0], &dst[0]);
        fill(&t[2], &dst[2]);
        ID3D11BlendState *b = blend(TRUE, 1, TRUE, D3D11_LOGIC_OP_AND_INVERTED, 0xf);
        ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        printf("  rt0:");
        show(&t[0], 0, 0);
        printf(" rt1:");
        show(&t[2], 0, 0);
        puts("");
        if (b) ID3D11BlendState_Release(b);
        /* (without IndependentBlendEnable: target 0's state for both) */
        fill(&t[0], &dst[0]);
        fill(&t[2], &dst[2]);
        b = blend(FALSE, 0, TRUE, D3D11_LOGIC_OP_OR, 0xf);
        ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        printf("  both or: rt0:");
        show(&t[0], 0, 0);
        printf(" rt1:");
        show(&t[2], 0, 0);
        puts("");
        if (b) ID3D11BlendState_Release(b);
    }

    puts("a logic op on an R8G8B8A8_UNORM target");
    {
        struct target u;
        if (!target(&u, DXGI_FORMAT_R8G8B8A8_UNORM, 4)) return 0;
        static const float f[4] = { 0.25f, 0.5f, 1.0f, 0.0f };
        static const UINT none[4] = { 0, 0, 0, 0 };
        constants(none, NULL, f);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &u.v, NULL);
        ID3D11DeviceContext_PSSetShader(ctx, psf, NULL, 0);
        for (int op = 2; op < 16; op += 8) {
            fill(&u, &dst[0]);
            ID3D11BlendState *b = blend(FALSE, 0, TRUE, op == 2 ? D3D11_LOGIC_OP_COPY : D3D11_LOGIC_OP_XOR, 0xf);
            ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            printf("  %s:", op == 2 ? "copy" : "xor");
            show(&u, 0, 0);
            puts("");
            if (b) ID3D11BlendState_Release(b);
        }
    }
    puts("done");
    return 0;
}
