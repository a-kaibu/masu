/* The pixel shader's inputs as the rasterizer interpolates them, to the last bit: every input of
 * every pixel (and sample) a draw covers written by the pixel shader into a raw UAV (u1), next
 * to SV_Coverage. The cases:
 *  - a triangle whose corners have different w: linear (perspective-correct), noperspective,
 *    nointerpolation (float and uint: which corner gives them), SV_Position (x y z w);
 *  - a quad of two triangles (a strip) over the target, w 1: a texture's coordinates as games
 *    draw them, with a viewport not at the origin too;
 *  - triangles the clipper cuts: by the left, top and right planes, by the near plane (depth
 *    clip on, perspective), by a user clip distance;
 *  - a geometry shader passing the triangle on;
 *  - a line list (x-major and y-major), a point list, the triangle in WIREFRAME;
 *  - a 4x multisampled target: pixel-rate, centroid, sample, noperspective centroid and
 *    sample, SV_Position at the sample (each sample-rate shader once per sample, its record
 *    at the sample's index).
 * Each record: x,y[sample]: the 4 words the shader wrote (asuint), then SV_Coverage.
 * WARP or hardware on the Windows runner.
 * libs: -ld3d11 -ld3dcompiler_47
 * lines: all
 * run: browser (sake's Direct3D 11 runs in the page: tests/winref/run-browser.mjs) */
#define COBJMACROS
#define INITGUID
#include "../probe.h"
#include <d3d11.h>
#include <d3dcompiler.h>

#define W 8
#define H 8
/* records: W x H pixels, 4 samples each, 5 words */
#define REC 5
#define WORDS (W * H * 4 * REC)
#define EMPTY 0xdeadbeefu

static ID3D11Device *dev;
static ID3D11DeviceContext *ctx;

static const char *SRC =
    "cbuffer V : register(b0) { float4 P[8]; float4 A[8]; float4 D[8]; };\n"
    "struct VO { float4 p : SV_Position; float4 a : A; uint k : K; };\n"
    "VO vs(uint id : SV_VertexID) { VO o; o.p = P[id]; o.a = A[id]; o.k = id * 0x101 + 7; return o; }\n"
    "struct VC { float4 p : SV_Position; float4 a : A; uint k : K; float c : SV_ClipDistance0; };\n"
    "VC vs_clip(uint id : SV_VertexID) { VC o; o.p = P[id]; o.a = A[id]; o.k = id * 0x101 + 7; o.c = D[id].x; return o; }\n"
    "[maxvertexcount(3)] void gs(triangle VO i[3], inout TriangleStream<VO> s) { s.Append(i[0]); s.Append(i[1]); s.Append(i[2]); }\n"
    "RWByteAddressBuffer u : register(u1);\n"
    "void put(float4 p, uint s, uint4 v, uint c) { uint b = (((uint)p.y * 8 + (uint)p.x) * 4 + s) * 20; u.Store4(b, v); u.Store(b + 16, c); }\n"
    "float4 ps_lin(float4 p : SV_Position, linear float4 a : A, uint c : SV_Coverage) : SV_Target { put(p, 0, asuint(a), c); return 0; }\n"
    "float4 ps_nop(float4 p : SV_Position, noperspective float4 a : A, uint c : SV_Coverage) : SV_Target { put(p, 0, asuint(a), c); return 0; }\n"
    "float4 ps_cen(float4 p : SV_Position, centroid float4 a : A, uint c : SV_Coverage) : SV_Target { put(p, 0, asuint(a), c); return 0; }\n"
    "float4 ps_nopcen(float4 p : SV_Position, noperspective centroid float4 a : A, uint c : SV_Coverage) : SV_Target { put(p, 0, asuint(a), c); return 0; }\n"
    "float4 ps_smp(float4 p : SV_Position, sample float4 a : A, uint s : SV_SampleIndex, uint c : SV_Coverage) : SV_Target { put(p, s, asuint(a), c); return 0; }\n"
    "float4 ps_nopsmp(float4 p : SV_Position, noperspective sample float4 a : A, uint s : SV_SampleIndex, uint c : SV_Coverage) : SV_Target { put(p, s, asuint(a), c); return 0; }\n"
    "float4 ps_flat(float4 p : SV_Position, nointerpolation float4 a : A, nointerpolation uint k : K, uint c : SV_Coverage) : SV_Target { put(p, 0, uint4(asuint(a.xyz), k), c); return 0; }\n"
    "float4 ps_pos(float4 p : SV_Position, uint c : SV_Coverage) : SV_Target { put(p, 0, asuint(p), c); return 0; }\n"
    "float4 ps_possmp(sample float4 p : SV_Position, uint s : SV_SampleIndex, uint c : SV_Coverage) : SV_Target { put(p, s, asuint(p), c); return 0; }\n"
    "float4 ps_poscen(centroid float4 p : SV_Position, uint c : SV_Coverage) : SV_Target { put(p, 0, asuint(p), c); return 0; }\n"
    "float4 ps_clip(float4 p : SV_Position, linear float4 a : A, float d : SV_ClipDistance0, uint c : SV_Coverage) : SV_Target { put(p, 0, uint4(asuint(a.xyz), asuint(d)), c); return 0; }\n";

static ID3DBlob *compile(const char *entry, const char *target) {
    ID3DBlob *b = NULL, *err = NULL;
    HRESULT hr = D3DCompile(SRC, strlen(SRC), "interp", NULL, NULL, entry, target, 0, 0, &b, &err);
    if (FAILED(hr)) printf("D3DCompile %s failed %#lx %s\n", entry, hr, err ? (char *)ID3D10Blob_GetBufferPointer(err) : "");
    if (err) ID3D10Blob_Release(err);
    return b;
}

enum { PS_LIN, PS_NOP, PS_CEN, PS_NOPCEN, PS_SMP, PS_NOPSMP, PS_FLAT, PS_POS, PS_POSSMP, PS_POSCEN, PS_CLIP, NPS };
static const char *PSN[NPS] = { "ps_lin", "ps_nop", "ps_cen", "ps_nopcen", "ps_smp", "ps_nopsmp", "ps_flat", "ps_pos", "ps_possmp", "ps_poscen", "ps_clip" };
static ID3D11PixelShader *pss[NPS];
static ID3D11VertexShader *vs, *vs_clip;
static ID3D11GeometryShader *gs;
static ID3D11Buffer *cb, *ub, *stage;
static ID3D11UnorderedAccessView *uav;
static ID3D11RenderTargetView *rt1, *rt4;
static ID3D11RasterizerState *solid, *wire;

/* the vertices (clip-space position, attribute, clip distance in x) */
static void vertices(const float (*p)[4], const float (*a)[4], const float *dist, int n) {
    float d[96];
    memset(d, 0, sizeof d);
    for (int i = 0; i < n && i < 8; i++) {
        memcpy(d + i * 4, p[i], 16);
        memcpy(d + 32 + i * 4, a[i], 16);
        if (dist) d[64 + i * 4] = dist[i];
    }
    ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)cb, 0, NULL, d, 0, 0);
}

/* one draw of `n` vertices, every record it wrote printed */
static void run(const char *name, int ps, int msaa, D3D11_PRIMITIVE_TOPOLOGY topo, int n, BOOL geometry, BOOL clipdist, BOOL wired, const D3D11_VIEWPORT *v) {
    printf("%s %s%s:\n", name, PSN[ps], msaa ? " 4x" : "");
    if (!pss[ps]) { puts("  no shader"); return; }
    static const UINT e[4] = { EMPTY, EMPTY, EMPTY, EMPTY };
    ID3D11DeviceContext_ClearUnorderedAccessViewUint(ctx, uav, e);
    ID3D11RenderTargetView *rt = msaa ? rt4 : rt1;
    ID3D11DeviceContext_OMSetRenderTargetsAndUnorderedAccessViews(ctx, 1, &rt, NULL, 1, 1, &uav, NULL);
    D3D11_VIEWPORT full = { 0, 0, W, H, 0, 1 };
    ID3D11DeviceContext_RSSetViewports(ctx, 1, v ? v : &full);
    ID3D11DeviceContext_RSSetState(ctx, wired ? wire : solid);
    ID3D11DeviceContext_IASetPrimitiveTopology(ctx, topo);
    ID3D11DeviceContext_VSSetShader(ctx, clipdist ? vs_clip : vs, NULL, 0);
    ID3D11DeviceContext_GSSetShader(ctx, geometry ? gs : NULL, NULL, 0);
    ID3D11DeviceContext_PSSetShader(ctx, pss[ps], NULL, 0);
    ID3D11DeviceContext_Draw(ctx, n, 0);
    ID3D11DeviceContext_GSSetShader(ctx, NULL, NULL, 0);
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)stage, (ID3D11Resource *)ub);
    D3D11_MAPPED_SUBRESOURCE m;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)stage, 0, D3D11_MAP_READ, 0, &m))) { puts("  map failed"); return; }
    const UINT *w = m.pData;
    for (int y = 0; y < H; y++)
        for (int x = 0; x < W; x++)
            for (int s = 0; s < 4; s++) {
                const UINT *r = w + ((y * W + x) * 4 + s) * REC;
                if (r[0] == EMPTY && r[1] == EMPTY && r[4] == EMPTY) continue;
                printf("  %d,%d[%d]: %08x %08x %08x %08x %x\n", x, y, s, r[0], r[1], r[2], r[3], r[4]);
            }
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)stage, 0);
}

int main(void) {
    D3D_FEATURE_LEVEL fl;
    if (FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx)) &&
        FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx))) {
        puts("skip no device");
        return 0;
    }
    if (fl < D3D_FEATURE_LEVEL_11_0) { puts("skip feature level below 11_0"); return 0; }
    ID3DBlob *b = compile("vs", "vs_5_0");
    if (!b) return 0;
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(b), ID3D10Blob_GetBufferSize(b), NULL, &vs);
    ID3D10Blob_Release(b);
    if ((b = compile("vs_clip", "vs_5_0"))) {
        ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(b), ID3D10Blob_GetBufferSize(b), NULL, &vs_clip);
        ID3D10Blob_Release(b);
    }
    if ((b = compile("gs", "gs_5_0"))) {
        ID3D11Device_CreateGeometryShader(dev, ID3D10Blob_GetBufferPointer(b), ID3D10Blob_GetBufferSize(b), NULL, &gs);
        ID3D10Blob_Release(b);
    }
    for (int i = 0; i < NPS; i++)
        if ((b = compile(PSN[i], "ps_5_0"))) {
            ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(b), ID3D10Blob_GetBufferSize(b), NULL, &pss[i]);
            ID3D10Blob_Release(b);
        }
    {
        D3D11_BUFFER_DESC bd = { 384, D3D11_USAGE_DEFAULT, D3D11_BIND_CONSTANT_BUFFER, 0, 0, 0 };
        ID3D11Device_CreateBuffer(dev, &bd, NULL, &cb);
        ID3D11DeviceContext_VSSetConstantBuffers(ctx, 0, 1, &cb);
    }
    {
        D3D11_BUFFER_DESC bd = { WORDS * 4, D3D11_USAGE_DEFAULT, D3D11_BIND_UNORDERED_ACCESS, 0, D3D11_RESOURCE_MISC_BUFFER_ALLOW_RAW_VIEWS, 0 };
        ID3D11Device_CreateBuffer(dev, &bd, NULL, &ub);
        D3D11_UNORDERED_ACCESS_VIEW_DESC ud;
        memset(&ud, 0, sizeof ud);
        ud.Format = DXGI_FORMAT_R32_TYPELESS;
        ud.ViewDimension = D3D11_UAV_DIMENSION_BUFFER;
        ud.Buffer.NumElements = WORDS;
        ud.Buffer.Flags = D3D11_BUFFER_UAV_FLAG_RAW;
        ID3D11Device_CreateUnorderedAccessView(dev, (ID3D11Resource *)ub, &ud, &uav);
        D3D11_BUFFER_DESC sd = { WORDS * 4, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ, 0, 0 };
        ID3D11Device_CreateBuffer(dev, &sd, NULL, &stage);
    }
    for (int k = 0; k < 2; k++) {
        D3D11_TEXTURE2D_DESC td = { W, H, 1, 1, DXGI_FORMAT_R8G8B8A8_UNORM, { k ? 4 : 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET, 0, 0 };
        ID3D11Texture2D *t;
        if (FAILED(ID3D11Device_CreateTexture2D(dev, &td, NULL, &t))) { puts("skip no target"); return 0; }
        ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)t, NULL, k ? &rt4 : &rt1);
    }
    {
        D3D11_RASTERIZER_DESC rd = { D3D11_FILL_SOLID, D3D11_CULL_NONE, FALSE, 0, 0, 0, TRUE, FALSE, FALSE, FALSE };
        ID3D11Device_CreateRasterizerState(dev, &rd, &solid);
        rd.FillMode = D3D11_FILL_WIREFRAME;
        ID3D11Device_CreateRasterizerState(dev, &rd, &wire);
    }

    /* a triangle whose corners have different w (clip-space positions times w) */
    static const float tp[3][4] = { { -0.9f, -0.8f, 0.3f, 1.0f }, { 0.4f, 1.8f, 0.2f, 2.0f }, { 0.4f, -0.35f, 0.3f, 0.5f } };
    static const float ta[3][4] = { { 0.1f, 1.7f, -3.25f, 100.0f }, { 0.73f, -0.2f, 7.5f, 0.001f }, { 0.31f, 0.9f, 0.0f, -42.5f } };
    vertices(tp, ta, NULL, 3);
    int tri[] = { PS_LIN, PS_NOP, PS_CEN, PS_SMP, PS_FLAT, PS_POS };
    for (int i = 0; i < 6; i++) run("triangle", tri[i], 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3, FALSE, FALSE, FALSE, NULL);
    run("triangle, geometry shader", PS_LIN, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3, TRUE, FALSE, FALSE, NULL);
    run("triangle, geometry shader", PS_FLAT, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3, TRUE, FALSE, FALSE, NULL);
    run("triangle, wireframe", PS_LIN, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3, FALSE, FALSE, TRUE, NULL);
    int ms[] = { PS_LIN, PS_CEN, PS_NOPCEN, PS_SMP, PS_NOPSMP, PS_POS, PS_POSSMP, PS_POSCEN };
    for (int i = 0; i < 8; i++) run("triangle", ms[i], 1, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3, FALSE, FALSE, FALSE, NULL);

    /* the same triangle, its corners turned (which corner the setup starts from) */
    {
        static const float rp[3][4] = { { 0.4f, 1.8f, 0.2f, 2.0f }, { 0.4f, -0.35f, 0.3f, 0.5f }, { -0.9f, -0.8f, 0.3f, 1.0f } };
        static const float ra[3][4] = { { 0.73f, -0.2f, 7.5f, 0.001f }, { 0.31f, 0.9f, 0.0f, -42.5f }, { 0.1f, 1.7f, -3.25f, 100.0f } };
        vertices(rp, ra, NULL, 3);
        run("triangle turned", PS_LIN, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3, FALSE, FALSE, FALSE, NULL);
        run("triangle turned", PS_FLAT, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3, FALSE, FALSE, FALSE, NULL);
    }

    /* a quad over the target as a strip, texture coordinates 0..1 */
    {
        static const float qp[4][4] = { { -1, 1, 0.5f, 1 }, { 1, 1, 0.5f, 1 }, { -1, -1, 0.5f, 1 }, { 1, -1, 0.5f, 1 } };
        static const float qa[4][4] = { { 0, 0, 0.25f, 1 }, { 1, 0, 0.75f, 1 }, { 0, 1, 0.5f, 0 }, { 1, 1, 0.125f, 0 } };
        vertices(qp, qa, NULL, 4);
        run("quad", PS_LIN, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, 4, FALSE, FALSE, FALSE, NULL);
        run("quad", PS_NOP, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, 4, FALSE, FALSE, FALSE, NULL);
        D3D11_VIEWPORT v = { 0.5f, 1.25f, 6.5f, 5.75f, 0, 1 };
        run("quad, viewport 0.5 1.25 6.5 5.75", PS_LIN, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, 4, FALSE, FALSE, FALSE, &v);
        run("quad, viewport 0.5 1.25 6.5 5.75", PS_POS, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, 4, FALSE, FALSE, FALSE, &v);
        run("quad", PS_LIN, 1, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, 4, FALSE, FALSE, FALSE, NULL);
    }

    /* triangles the clipper cuts */
    {
        static const float cp[3][4] = { { -1.7f, -0.3f, 0.5f, 1 }, { 0.6f, 1.9f, 0.5f, 1 }, { 1.4f, -1.2f, 0.5f, 1 } };
        static const float ca[3][4] = { { 0.2f, 3.0f, -1.5f, 0.6f }, { 0.9f, -2.0f, 0.25f, 0.1f }, { 0.45f, 0.7f, 5.5f, 0.35f } };
        vertices(cp, ca, NULL, 3);
        run("clipped left top right", PS_LIN, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3, FALSE, FALSE, FALSE, NULL);
        run("clipped left top right", PS_NOP, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3, FALSE, FALSE, FALSE, NULL);
        run("clipped left top right", PS_FLAT, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3, FALSE, FALSE, FALSE, NULL);
        run("clipped left top right", PS_POS, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3, FALSE, FALSE, FALSE, NULL);
        static const float np[3][4] = { { -0.8f, -0.8f, -0.5f, 1 }, { 0.0f, 1.35f, 0.75f, 1.5f }, { 0.9f, -0.6f, 0.9f, 1 } };
        vertices(np, ca, NULL, 3);
        run("clipped near", PS_LIN, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3, FALSE, FALSE, FALSE, NULL);
        run("clipped near", PS_NOP, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3, FALSE, FALSE, FALSE, NULL);
        run("clipped near", PS_POS, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3, FALSE, FALSE, FALSE, NULL);
        static const float dist[3] = { -0.5f, 1.0f, 0.75f };
        vertices(tp, ta, dist, 3);
        run("clip distance", PS_CLIP, 0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3, FALSE, TRUE, FALSE, NULL);
    }

    /* lines and points */
    {
        static const float lp[4][4] = { { -0.95f, -0.6f, 0.5f, 1 }, { 1.6f, 0.9f, 0.25f, 2 }, { -0.3f, 0.95f, 0.5f, 0.5f }, { 0.1f, -0.9f, 0.5f, 1 } };
        static const float la[4][4] = { { 0.1f, 1.7f, -3.25f, 100.0f }, { 0.73f, -0.2f, 7.5f, 0.001f }, { 0.31f, 0.9f, 0.0f, -42.5f }, { 0.6f, 0.15f, 2.0f, 9.0f } };
        vertices(lp, la, NULL, 4);
        run("lines", PS_LIN, 0, D3D11_PRIMITIVE_TOPOLOGY_LINELIST, 4, FALSE, FALSE, FALSE, NULL);
        run("lines", PS_NOP, 0, D3D11_PRIMITIVE_TOPOLOGY_LINELIST, 4, FALSE, FALSE, FALSE, NULL);
        run("lines", PS_FLAT, 0, D3D11_PRIMITIVE_TOPOLOGY_LINELIST, 4, FALSE, FALSE, FALSE, NULL);
        run("lines", PS_POS, 0, D3D11_PRIMITIVE_TOPOLOGY_LINELIST, 4, FALSE, FALSE, FALSE, NULL);
        run("points", PS_LIN, 0, D3D11_PRIMITIVE_TOPOLOGY_POINTLIST, 4, FALSE, FALSE, FALSE, NULL);
        run("points", PS_POS, 0, D3D11_PRIMITIVE_TOPOLOGY_POINTLIST, 4, FALSE, FALSE, FALSE, NULL);
    }
    puts("done");
    return 0;
}
