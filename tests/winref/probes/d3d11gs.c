/* Direct3D 11 geometry shaders, stream output and what the pipeline statistics count:
 *  - zero-sized viewports (width, height or both 0): pixels written, samples counted, the
 *    clipper's counts, RSGetViewports (a clockwise full-target triangle: not culled);
 *  - PIPELINE_STATISTICS of single triangles placed in clip space (inside, outside, across an
 *    edge, far outside, across the near plane, behind it, a negative w, degenerate, culled),
 *    of indexed draws reusing vertices, strips, lines and points;
 *  - the vertices a geometry shader is given (ids by SV_VertexID) for each topology (lists,
 *    strips, adjacency, a cut strip, instances, GS instancing), with SV_PrimitiveID,
 *    captured by stream output;
 *  - how stream output writes strips a geometry shader appends (and cuts) as lists;
 *  - the rasterized strip: which vertex a flat (nointerpolation) value comes from, culling of
 *    its odd triangles;
 *  - stream output of a vertex shader: overflow (SO_STATISTICS, SO_OVERFLOW_PREDICATE),
 *    appending with offset -1, declarations with gaps / start components / two slots,
 *    DrawAuto, rasterization alongside, SOGetTargets, two streams;
 *  - CreateGeometryShaderWithStreamOutput's errors.
 * WARP or hardware on the Windows runner.
 * libs: -ld3d11 -ld3dcompiler_47
 * run: browser (sake's Direct3D 11 runs in the page: tests/winref/run-browser.mjs) */
#define COBJMACROS
#define INITGUID
#include "../probe.h"
#include <d3d11.h>
#include <d3dcompiler.h>

#define W 64
#define H 64

static ID3D11Device *dev;
static ID3D11DeviceContext *ctx;

static const char *SRC =
    "struct V { float4 p : SV_Position; uint id : ID; };\n"
    "V vs_id(uint id : SV_VertexID, uint inst : SV_InstanceID) { V o; o.p = float4(0, 0, 0.5, 1); o.id = id + inst * 100; return o; }\n"
    "float4 vs_full(uint id : SV_VertexID) : SV_Position { return float4((id >> 1) * 4.0 - 1.0, (id & 1) * 4.0 - 1.0, 0.5, 1); }\n"
    "float4 vs_pos(float4 p : POS) : SV_Position { return p; }\n"
    "struct S { float4 p : SV_Position; float4 c : COL; float2 t : TEX; };\n"
    "S vs_so(float4 p : POS, uint id : SV_VertexID) { S o; o.p = p; o.c = float4(id, id * 2, id * 3, id * 4); o.t = float2(id + 0.5, -1.0 * id); return o; }\n"
    "float4 ps(float4 p : SV_Position) : SV_Target { return float4(1, 0, 1, 1); }\n"
    "struct G { uint4 a : A; };\n"
    "[maxvertexcount(1)] void gs_point(point V v[1], uint pid : SV_PrimitiveID, inout PointStream<G> s) { G o; o.a = uint4(v[0].id, 0, 0, pid); s.Append(o); }\n"
    "[maxvertexcount(1)] void gs_line(line V v[2], uint pid : SV_PrimitiveID, inout PointStream<G> s) { G o; o.a = uint4(v[0].id, v[1].id, 0, pid); s.Append(o); }\n"
    "[maxvertexcount(1)] void gs_tri(triangle V v[3], uint pid : SV_PrimitiveID, inout PointStream<G> s) { G o; o.a = uint4(v[0].id, v[1].id, v[2].id, pid); s.Append(o); }\n"
    "[maxvertexcount(2)] void gs_lineadj(lineadj V v[4], uint pid : SV_PrimitiveID, inout PointStream<G> s) { G o; o.a = uint4(v[0].id, v[1].id, v[2].id, v[3].id); s.Append(o); o.a = uint4(pid, 0, 0, 0); s.Append(o); }\n"
    "[maxvertexcount(2)] void gs_triadj(triangleadj V v[6], uint pid : SV_PrimitiveID, inout PointStream<G> s) { G o; o.a = uint4(v[0].id, v[1].id, v[2].id, v[3].id); s.Append(o); o.a = uint4(v[4].id, v[5].id, pid, 0); s.Append(o); }\n"
    "[instance(3)] [maxvertexcount(1)] void gs_inst(triangle V v[3], uint pid : SV_PrimitiveID, uint gi : SV_GSInstanceID, inout PointStream<G> s) { G o; o.a = uint4(v[0].id, v[1].id, gi, pid); s.Append(o); }\n"
    "struct O { uint a : A; };\n"
    "[maxvertexcount(8)] void gs_strip(point V v[1], inout TriangleStream<O> s) { O o; for (uint i = 0; i < 5; i++) { o.a = i; s.Append(o); } s.RestartStrip(); for (uint j = 10; j < 13; j++) { o.a = j; s.Append(o); } }\n"
    "[maxvertexcount(6)] void gs_lstrip(point V v[1], inout LineStream<O> s) { O o; for (uint i = 0; i < 3; i++) { o.a = i; s.Append(o); } s.RestartStrip(); o.a = 9; s.Append(o); s.RestartStrip(); o.a = 20; s.Append(o); o.a = 21; s.Append(o); }\n"
    "[maxvertexcount(4)] void gs_few(point V v[1], inout TriangleStream<O> s) { O o; o.a = 1; s.Append(o); o.a = 2; s.Append(o); }\n"
    "struct F { float4 p : SV_Position; nointerpolation uint k : K; };\n"
    "[maxvertexcount(4)] void gs_quad(point V v[1], inout TriangleStream<F> s) { F o; float2 c[4] = { float2(-1, -1), float2(-1, 1), float2(1, -1), float2(1, 1) };\n"
    "  for (uint i = 0; i < 4; i++) { o.p = float4(c[i], 0.5, 1); o.k = i + 1; s.Append(o); } }\n"
    "float4 ps_flat(float4 p : SV_Position, nointerpolation uint k : K) : SV_Target { return float4(k == 1, k == 2, k == 3, 1) + (k == 4) * float4(1, 1, 1, 0); }\n"
    "struct T { uint a : A; };\n"
    "[maxvertexcount(4)] void gs_two(point V v[1], inout PointStream<T> s0, inout PointStream<T> s1) { T o; o.a = v[0].id; s0.Append(o); o.a = v[0].id + 50; s1.Append(o); s1.Append(o); }\n";

static ID3DBlob *compile(const char *entry, const char *target) {
    ID3DBlob *b = NULL, *err = NULL;
    HRESULT hr = D3DCompile(SRC, strlen(SRC), "gs", NULL, NULL, entry, target, 0, 0, &b, &err);
    if (FAILED(hr)) printf("D3DCompile %s failed %#lx %s\n", entry, hr, err ? (char *)ID3D10Blob_GetBufferPointer(err) : "");
    if (err) ID3D10Blob_Release(err);
    return b;
}

static void viewport(float x, float y, float w, float h) {
    D3D11_VIEWPORT vp = { x, y, w, h, 0, 1 };
    ID3D11DeviceContext_RSSetViewports(ctx, 1, &vp);
}

static HRESULT wait(ID3D11Asynchronous *q, void *data, UINT size) {
    HRESULT hr;
    int n = 0;
    while ((hr = ID3D11DeviceContext_GetData(ctx, q, data, size, 0)) == S_FALSE && n < 1000000) n++;
    return hr;
}

static ID3D11Query *query(D3D11_QUERY t) {
    D3D11_QUERY_DESC d = { t, 0 };
    ID3D11Query *q = NULL;
    ID3D11Device_CreateQuery(dev, &d, &q);
    return q;
}

static ID3D11Buffer *buffer(UINT size, UINT bind, const void *data) {
    D3D11_BUFFER_DESC bd = { size, D3D11_USAGE_DEFAULT, bind, 0, 0, 0 };
    D3D11_SUBRESOURCE_DATA init = { data, 0, 0 };
    ID3D11Buffer *b = NULL;
    HRESULT hr = ID3D11Device_CreateBuffer(dev, &bd, data ? &init : NULL, &b);
    if (FAILED(hr)) printf("CreateBuffer %u %#x failed %#lx\n", size, bind, hr);
    return b;
}

/* a stream output buffer of n dwords, each 0xcdcdcdcd */
static ID3D11Buffer *so_buffer(UINT n) {
    UINT *fill = malloc(n * 4);
    for (UINT i = 0; i < n; i++) fill[i] = 0xcdcdcdcd;
    ID3D11Buffer *b = buffer(n * 4, D3D11_BIND_STREAM_OUTPUT | D3D11_BIND_VERTEX_BUFFER, fill);
    free(fill);
    return b;
}

/* the first n dwords of a buffer, hex, `per` to a line */
static void dump(const char *label, ID3D11Buffer *b, UINT n, UINT per) {
    D3D11_BUFFER_DESC bd;
    ID3D11Buffer_GetDesc(b, &bd);
    bd.Usage = D3D11_USAGE_STAGING;
    bd.BindFlags = 0;
    bd.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    bd.MiscFlags = 0;
    ID3D11Buffer *s = NULL;
    ID3D11Device_CreateBuffer(dev, &bd, NULL, &s);
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)s, (ID3D11Resource *)b);
    D3D11_MAPPED_SUBRESOURCE m;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)s, 0, D3D11_MAP_READ, 0, &m))) { printf("  %s: Map failed\n", label); return; }
    const UINT *p = m.pData;
    printf("  %s:", label);
    for (UINT i = 0; i < n; i++) printf("%s%x", i % per == 0 ? (i ? "\n   " : " ") : " ", p[i]);
    puts("");
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)s, 0);
    ID3D11Buffer_Release(s);
}

static void print_stats(const char *label, const D3D11_QUERY_DATA_PIPELINE_STATISTICS *s) {
    printf("  %s: IA %llu %llu VS %llu GS %llu %llu C %llu %llu PS %llu\n", label, (unsigned long long)s->IAVertices, (unsigned long long)s->IAPrimitives,
        (unsigned long long)s->VSInvocations, (unsigned long long)s->GSInvocations, (unsigned long long)s->GSPrimitives, (unsigned long long)s->CInvocations,
        (unsigned long long)s->CPrimitives, (unsigned long long)s->PSInvocations);
}

static ID3D11Texture2D *rt, *staging;
static ID3D11RenderTargetView *rtv;

/* pixels of the target that are not 0 */
static int written(void) {
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)staging, (ID3D11Resource *)rt);
    D3D11_MAPPED_SUBRESOURCE m;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)staging, 0, D3D11_MAP_READ, 0, &m))) return -1;
    int n = 0;
    for (int y = 0; y < H; y++)
        for (int x = 0; x < W; x++) n += ((const UINT *)((const char *)m.pData + y * m.RowPitch))[x] != 0;
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)staging, 0);
    return n;
}

static void image(const char *name, int w, int h) {
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)staging, (ID3D11Resource *)rt);
    D3D11_MAPPED_SUBRESOURCE m;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)staging, 0, D3D11_MAP_READ, 0, &m))) return;
    /* RGBA -> the probe's B, G, R, x */
    static UINT px[W * H];
    for (int y = 0; y < h; y++)
        for (int x = 0; x < w; x++) {
            UINT c = ((const UINT *)((const char *)m.pData + y * m.RowPitch))[x];
            px[y * w + x] = (c & 0xff) << 16 | (c & 0xff00) | (c >> 16 & 0xff);
        }
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)staging, 0);
    probe_image(name, px, w, h, w * 4, 0);
}

static void clear(void) {
    float c[4] = { 0, 0, 0, 0 };
    ID3D11DeviceContext_ClearRenderTargetView(ctx, rtv, c);
}

static ID3D11GeometryShader *so_shader(ID3DBlob *b, const D3D11_SO_DECLARATION_ENTRY *e, UINT n, const UINT *strides, UINT ns, UINT raster, HRESULT *hr) {
    ID3D11GeometryShader *g = NULL;
    *hr = ID3D11Device_CreateGeometryShaderWithStreamOutput(dev, ID3D10Blob_GetBufferPointer(b), ID3D10Blob_GetBufferSize(b), e, n, strides, ns, raster, NULL, &g);
    return g;
}

int main(void) {
    D3D_FEATURE_LEVEL fl;
    if (FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx)) &&
        FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx))) {
        puts("skip no device");
        return 0;
    }
    if (fl < D3D_FEATURE_LEVEL_11_0) { puts("skip feature level below 11_0"); return 0; }
    D3D11_TEXTURE2D_DESC td = { W, H, 1, 1, DXGI_FORMAT_R8G8B8A8_UNORM, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET, 0, 0 };
    ID3D11Device_CreateTexture2D(dev, &td, NULL, &rt);
    td.Usage = D3D11_USAGE_STAGING;
    td.BindFlags = 0;
    td.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    ID3D11Device_CreateTexture2D(dev, &td, NULL, &staging);
    ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)rt, NULL, &rtv);
    ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &rtv, NULL);

    static const char *names[] = { "vs_id", "vs_full", "vs_pos", "vs_so", "ps", "gs_point", "gs_line", "gs_tri", "gs_lineadj", "gs_triadj", "gs_inst", "gs_strip", "gs_lstrip", "gs_few", "gs_quad", "ps_flat", "gs_two" };
    static const char *targets[] = { "vs_4_0", "vs_4_0", "vs_4_0", "vs_4_0", "ps_4_0", "gs_4_0", "gs_4_0", "gs_4_0", "gs_4_0", "gs_4_0", "gs_5_0", "gs_4_0", "gs_4_0", "gs_4_0", "gs_4_0", "ps_4_0", "gs_5_0" };
    enum { VS_ID, VS_FULL, VS_POS, VS_SO, PS, GS_POINT, GS_LINE, GS_TRI, GS_LINEADJ, GS_TRIADJ, GS_INST, GS_STRIP, GS_LSTRIP, GS_FEW, GS_QUAD, PS_FLAT, GS_TWO, N };
    ID3DBlob *bl[N];
    for (int i = 0; i < N; i++)
        if (!(bl[i] = compile(names[i], targets[i]))) return 0;
    ID3D11VertexShader *vs[4];
    for (int i = 0; i < 4; i++) ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(bl[i]), ID3D10Blob_GetBufferSize(bl[i]), NULL, &vs[i]);
    ID3D11PixelShader *ps = NULL, *ps_flat = NULL;
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[PS]), ID3D10Blob_GetBufferSize(bl[PS]), NULL, &ps);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[PS_FLAT]), ID3D10Blob_GetBufferSize(bl[PS_FLAT]), NULL, &ps_flat);
    ID3D11InputLayout *pos_layout = NULL;
    {
        D3D11_INPUT_ELEMENT_DESC e = { "POS", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 0, D3D11_INPUT_PER_VERTEX_DATA, 0 };
        ID3D11Device_CreateInputLayout(dev, &e, 1, ID3D10Blob_GetBufferPointer(bl[VS_POS]), ID3D10Blob_GetBufferSize(bl[VS_POS]), &pos_layout);
    }
    ID3D11Query *occ = query(D3D11_QUERY_OCCLUSION), *st = query(D3D11_QUERY_PIPELINE_STATISTICS), *sos = query(D3D11_QUERY_SO_STATISTICS);
    ID3D11Predicate *ovf = NULL;
    {
        D3D11_QUERY_DESC d = { D3D11_QUERY_SO_OVERFLOW_PREDICATE, 0 };
        ID3D11Device_CreatePredicate(dev, &d, &ovf);
    }
    if (!occ || !st || !sos || !ovf) { puts("skip no queries"); return 0; }

    puts("zero-sized viewports, a full-target triangle");
    ID3D11DeviceContext_VSSetShader(ctx, vs[VS_FULL], NULL, 0);
    ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
    ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    {
        static const float vps[][4] = { { 0, 0, 0, 64 }, { 0, 0, 64, 0 }, { 0, 0, 0, 0 }, { 10, 10, 0, 20 }, { 10, 10, 20, 0 }, { 0, 0, 0.25f, 64 }, { 0, 0, 64, 64 } };
        for (int i = 0; i < 7; i++) {
            clear();
            viewport(vps[i][0], vps[i][1], vps[i][2], vps[i][3]);
            D3D11_VIEWPORT back[2] = { 0 };
            UINT n = 2;
            ID3D11DeviceContext_RSGetViewports(ctx, &n, back);
            ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)occ);
            ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)st);
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)st);
            ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)occ);
            UINT64 v = 0;
            D3D11_QUERY_DATA_PIPELINE_STATISTICS s;
            wait((ID3D11Asynchronous *)occ, &v, 8);
            wait((ID3D11Asynchronous *)st, &s, sizeof s);
            printf("  %g %g %g %g (back: %u, %g %g %g %g): pixels %d samples %llu C %llu %llu PS %llu\n", vps[i][0], vps[i][1], vps[i][2], vps[i][3], n, back[0].TopLeftX, back[0].TopLeftY,
                back[0].Width, back[0].Height, written(), (unsigned long long)v, (unsigned long long)s.CInvocations, (unsigned long long)s.CPrimitives, (unsigned long long)s.PSInvocations);
        }
    }

    puts("pipeline statistics of triangles in clip space (viewport 64x64, CULL_BACK)");
    viewport(0, 0, W, H);
    ID3D11DeviceContext_VSSetShader(ctx, vs[VS_POS], NULL, 0);
    ID3D11DeviceContext_IASetInputLayout(ctx, pos_layout);
    {
        static const struct { const char *name; float v[3][4]; } tris[] = {
            { "inside", { { -0.5f, -0.5f, 0.5f, 1 }, { -0.5f, 0.5f, 0.5f, 1 }, { 0.5f, -0.5f, 0.5f, 1 } } },
            { "inside, counterclockwise", { { -0.5f, -0.5f, 0.5f, 1 }, { 0.5f, -0.5f, 0.5f, 1 }, { -0.5f, 0.5f, 0.5f, 1 } } },
            { "outside right", { { 2, -0.5f, 0.5f, 1 }, { 2, 0.5f, 0.5f, 1 }, { 3, -0.5f, 0.5f, 1 } } },
            { "across the right edge", { { -0.5f, -0.5f, 0.5f, 1 }, { -0.5f, 0.5f, 0.5f, 1 }, { 1.5f, -0.5f, 0.5f, 1 } } },
            { "across two edges", { { -2, -0.5f, 0.5f, 1 }, { -0.5f, 2, 0.5f, 1 }, { 2, -2, 0.5f, 1 } } },
            { "far outside the guard band", { { -0.5f, -0.5f, 0.5f, 1 }, { -0.5f, 0.5f, 0.5f, 1 }, { 100000, -0.5f, 0.5f, 1 } } },
            { "across the near plane", { { -0.5f, -0.5f, -0.5f, 1 }, { -0.5f, 0.5f, 0.5f, 1 }, { 0.5f, -0.5f, 0.5f, 1 } } },
            { "across the far plane", { { -0.5f, -0.5f, 1.5f, 1 }, { -0.5f, 0.5f, 0.5f, 1 }, { 0.5f, -0.5f, 0.5f, 1 } } },
            { "behind the near plane", { { -0.5f, -0.5f, -0.5f, 1 }, { -0.5f, 0.5f, -0.5f, 1 }, { 0.5f, -0.5f, -0.5f, 1 } } },
            { "one w negative", { { -0.5f, -0.5f, 0.5f, -1 }, { -0.5f, 0.5f, 0.5f, 1 }, { 0.5f, -0.5f, 0.5f, 1 } } },
            { "all w negative", { { 0.5f, 0.5f, -0.5f, -1 }, { 0.5f, -0.5f, -0.5f, -1 }, { -0.5f, 0.5f, -0.5f, -1 } } },
            { "two vertices equal", { { -0.5f, -0.5f, 0.5f, 1 }, { -0.5f, -0.5f, 0.5f, 1 }, { 0.5f, -0.5f, 0.5f, 1 } } },
            { "on a line", { { -0.5f, -0.5f, 0.5f, 1 }, { 0, 0, 0.5f, 1 }, { 0.5f, 0.5f, 0.5f, 1 } } },
            { "smaller than a pixel", { { 0.001f, 0.001f, 0.5f, 1 }, { 0.001f, 0.002f, 0.5f, 1 }, { 0.002f, 0.001f, 0.5f, 1 } } },
            { "w zero", { { -0.5f, -0.5f, 0.5f, 0 }, { -0.5f, 0.5f, 0.5f, 1 }, { 0.5f, -0.5f, 0.5f, 1 } } },
        };
        for (int i = 0; i < (int)(sizeof tris / sizeof tris[0]); i++) {
            ID3D11Buffer *vb = buffer(sizeof tris[i].v, D3D11_BIND_VERTEX_BUFFER, tris[i].v);
            UINT stride = 16, off = 0;
            ID3D11DeviceContext_IASetVertexBuffers(ctx, 0, 1, &vb, &stride, &off);
            D3D11_QUERY_DATA_PIPELINE_STATISTICS s;
            ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)st);
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)st);
            wait((ID3D11Asynchronous *)st, &s, sizeof s);
            print_stats(tris[i].name, &s);
            ID3D11Buffer_Release(vb);
        }
    }

    puts("pipeline statistics of indexed draws, strips, lines and points");
    {
        static const float quad[6][4] = { { -0.5f, -0.5f, 0.5f, 1 }, { -0.5f, 0.5f, 0.5f, 1 }, { 0.5f, -0.5f, 0.5f, 1 }, { 0.5f, 0.5f, 0.5f, 1 }, { 2, 2, 0.5f, 1 }, { 3, 2, 0.5f, 1 } };
        static const USHORT idx[] = { 0, 1, 2, 2, 1, 3, 0, 1, 2, 0, 1, 2, 3, 3, 3, 0, 0xffff, 1, 2, 3 };
        ID3D11Buffer *vb = buffer(sizeof quad, D3D11_BIND_VERTEX_BUFFER, quad), *ib = buffer(sizeof idx, D3D11_BIND_INDEX_BUFFER, idx);
        UINT stride = 16, off = 0;
        ID3D11DeviceContext_IASetVertexBuffers(ctx, 0, 1, &vb, &stride, &off);
        ID3D11DeviceContext_IASetIndexBuffer(ctx, ib, DXGI_FORMAT_R16_UINT, 0);
        static const struct { const char *name; D3D11_PRIMITIVE_TOPOLOGY t; int indexed; UINT count, start, inst; } c[] = {
            { "indexed 0 1 2 2 1 3", D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 1, 6, 0, 1 },
            { "indexed 0 1 2 2 1 3 0 1 2 0 1 2", D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 1, 12, 0, 1 },
            { "indexed 3 3 3 (degenerate)", D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 1, 3, 12, 1 },
            { "indexed twice instanced", D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 1, 6, 0, 2 },
            { "indexed strip 0 0xffff 1 2 3", D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, 1, 5, 15, 1 },
            { "indexed strip 1 2 3 0 1 2", D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, 1, 6, 18, 1 },
            { "strip of 4", D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, 0, 4, 0, 1 },
            { "strip of 6 (2 outside)", D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, 0, 6, 0, 1 },
            { "list of 5 (one left over)", D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 0, 5, 0, 1 },
            { "line list of 6", D3D11_PRIMITIVE_TOPOLOGY_LINELIST, 0, 6, 0, 1 },
            { "line strip of 6", D3D11_PRIMITIVE_TOPOLOGY_LINESTRIP, 0, 6, 0, 1 },
            { "point list of 6", D3D11_PRIMITIVE_TOPOLOGY_POINTLIST, 0, 6, 0, 1 },
            { "triangle list adjacency of 6", D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST_ADJ, 0, 6, 0, 1 },
        };
        for (int i = 0; i < (int)(sizeof c / sizeof c[0]); i++) {
            D3D11_QUERY_DATA_PIPELINE_STATISTICS s;
            ID3D11DeviceContext_IASetPrimitiveTopology(ctx, c[i].t);
            ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)st);
            if (c[i].indexed) ID3D11DeviceContext_DrawIndexedInstanced(ctx, c[i].count, c[i].inst, c[i].start, 0, 0);
            else ID3D11DeviceContext_DrawInstanced(ctx, c[i].count, c[i].inst, c[i].start, 0);
            ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)st);
            wait((ID3D11Asynchronous *)st, &s, sizeof s);
            print_stats(c[i].name, &s);
        }
        ID3D11Buffer_Release(vb);
        ID3D11Buffer_Release(ib);
    }

    puts("the vertices a geometry shader is given (v[i].id, SV_PrimitiveID by stream output)");
    ID3D11DeviceContext_VSSetShader(ctx, vs[VS_ID], NULL, 0);
    ID3D11DeviceContext_IASetInputLayout(ctx, NULL);
    ID3D11DeviceContext_PSSetShader(ctx, NULL, NULL, 0);
    {
        D3D11_SO_DECLARATION_ENTRY e = { 0, "A", 0, 0, 4, 0 };
        UINT stride = 16;
        static const struct { const char *name; int gs; D3D11_PRIMITIVE_TOPOLOGY t; UINT count, inst; int indexed; } c[] = {
            { "points", GS_POINT, D3D11_PRIMITIVE_TOPOLOGY_POINTLIST, 3, 1, 0 },
            { "line list", GS_LINE, D3D11_PRIMITIVE_TOPOLOGY_LINELIST, 6, 1, 0 },
            { "line strip", GS_LINE, D3D11_PRIMITIVE_TOPOLOGY_LINESTRIP, 5, 1, 0 },
            { "triangle list", GS_TRI, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 6, 1, 0 },
            { "triangle strip", GS_TRI, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, 6, 1, 0 },
            { "triangle strip, 2 instances", GS_TRI, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, 4, 2, 0 },
            { "indexed strip 0 1 2 3 cut 4 5 6 7", GS_TRI, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, 9, 1, 1 },
            { "line list adjacency", GS_LINEADJ, D3D11_PRIMITIVE_TOPOLOGY_LINELIST_ADJ, 8, 1, 0 },
            { "line strip adjacency", GS_LINEADJ, D3D11_PRIMITIVE_TOPOLOGY_LINESTRIP_ADJ, 6, 1, 0 },
            { "triangle list adjacency", GS_TRIADJ, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST_ADJ, 12, 1, 0 },
            { "triangle strip adjacency", GS_TRIADJ, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP_ADJ, 10, 1, 0 },
            { "triangle strip adjacency of 6", GS_TRIADJ, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP_ADJ, 6, 1, 0 },
            { "triangle strip adjacency of 8", GS_TRIADJ, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP_ADJ, 8, 1, 0 },
            { "GS instances 3", GS_INST, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 6, 1, 0 },
        };
        static const USHORT idx[] = { 0, 1, 2, 3, 0xffff, 4, 5, 6, 7 };
        ID3D11Buffer *ib = buffer(sizeof idx, D3D11_BIND_INDEX_BUFFER, idx);
        ID3D11DeviceContext_IASetIndexBuffer(ctx, ib, DXGI_FORMAT_R16_UINT, 0);
        for (int i = 0; i < (int)(sizeof c / sizeof c[0]); i++) {
            HRESULT hr;
            ID3D11GeometryShader *g = so_shader(bl[c[i].gs], &e, 1, &stride, 1, D3D11_SO_NO_RASTERIZED_STREAM, &hr);
            if (!g) { printf("  %s: CreateGeometryShaderWithStreamOutput %#lx\n", c[i].name, hr); continue; }
            ID3D11Buffer *b = so_buffer(64);
            UINT off = 0;
            ID3D11DeviceContext_SOSetTargets(ctx, 1, &b, &off);
            ID3D11DeviceContext_GSSetShader(ctx, g, NULL, 0);
            ID3D11DeviceContext_IASetPrimitiveTopology(ctx, c[i].t);
            D3D11_QUERY_DATA_PIPELINE_STATISTICS s;
            D3D11_QUERY_DATA_SO_STATISTICS so = { 7, 7 };
            ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)st);
            ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)sos);
            if (c[i].indexed) ID3D11DeviceContext_DrawIndexedInstanced(ctx, c[i].count, c[i].inst, 0, 0, 0);
            else ID3D11DeviceContext_DrawInstanced(ctx, c[i].count, c[i].inst, 0, 0);
            ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)sos);
            ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)st);
            wait((ID3D11Asynchronous *)st, &s, sizeof s);
            wait((ID3D11Asynchronous *)sos, &so, sizeof so);
            ID3D11DeviceContext_SOSetTargets(ctx, 0, NULL, NULL);
            print_stats(c[i].name, &s);
            printf("  written %llu needed %llu\n", (unsigned long long)so.NumPrimitivesWritten, (unsigned long long)so.PrimitivesStorageNeeded);
            dump("buffer", b, (UINT)(so.NumPrimitivesWritten < 16 ? so.NumPrimitivesWritten : 16) * 4, 4);
            ID3D11Buffer_Release(b);
            ID3D11GeometryShader_Release(g);
        }
        ID3D11Buffer_Release(ib);
    }

    puts("strips a geometry shader appends, as stream output writes them");
    {
        D3D11_SO_DECLARATION_ENTRY e = { 0, "A", 0, 0, 1, 0 };
        UINT stride = 4;
        static const struct { const char *name; int gs; } c[] = { { "triangle strip 0..4 cut 10..12", GS_STRIP }, { "line strip 0..2 cut 9 cut 20 21", GS_LSTRIP }, { "two vertices of a triangle strip", GS_FEW } };
        ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_POINTLIST);
        for (int i = 0; i < 3; i++) {
            HRESULT hr;
            ID3D11GeometryShader *g = so_shader(bl[c[i].gs], &e, 1, &stride, 1, D3D11_SO_NO_RASTERIZED_STREAM, &hr);
            if (!g) { printf("  %s: %#lx\n", c[i].name, hr); continue; }
            ID3D11Buffer *b = so_buffer(64);
            UINT off = 0;
            ID3D11DeviceContext_SOSetTargets(ctx, 1, &b, &off);
            ID3D11DeviceContext_GSSetShader(ctx, g, NULL, 0);
            D3D11_QUERY_DATA_PIPELINE_STATISTICS s;
            D3D11_QUERY_DATA_SO_STATISTICS so = { 7, 7 };
            ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)st);
            ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)sos);
            ID3D11DeviceContext_Draw(ctx, 2, 0);
            ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)sos);
            ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)st);
            wait((ID3D11Asynchronous *)st, &s, sizeof s);
            wait((ID3D11Asynchronous *)sos, &so, sizeof so);
            ID3D11DeviceContext_SOSetTargets(ctx, 0, NULL, NULL);
            print_stats(c[i].name, &s);
            printf("  written %llu needed %llu\n", (unsigned long long)so.NumPrimitivesWritten, (unsigned long long)so.PrimitivesStorageNeeded);
            dump("buffer", b, 40, 10);
            ID3D11Buffer_Release(b);
            ID3D11GeometryShader_Release(g);
        }
    }

    puts("a rasterized strip of a geometry shader (flat K per vertex: 1 red 2 green 3 blue 4 white)");
    {
        ID3D11GeometryShader *g = NULL;
        ID3D11Device_CreateGeometryShader(dev, ID3D10Blob_GetBufferPointer(bl[GS_QUAD]), ID3D10Blob_GetBufferSize(bl[GS_QUAD]), NULL, &g);
        ID3D11DeviceContext_GSSetShader(ctx, g, NULL, 0);
        ID3D11DeviceContext_PSSetShader(ctx, ps_flat, NULL, 0);
        ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_POINTLIST);
        viewport(0, 0, 8, 8);
        for (int cull = 0; cull < 3; cull++) {
            D3D11_RASTERIZER_DESC rd = { D3D11_FILL_SOLID, cull == 0 ? D3D11_CULL_NONE : cull == 1 ? D3D11_CULL_BACK : D3D11_CULL_FRONT, FALSE, 0, 0, 0, TRUE, FALSE, FALSE, FALSE };
            ID3D11RasterizerState *rs = NULL;
            ID3D11Device_CreateRasterizerState(dev, &rd, &rs);
            ID3D11DeviceContext_RSSetState(ctx, rs);
            clear();
            D3D11_QUERY_DATA_PIPELINE_STATISTICS s;
            ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)st);
            ID3D11DeviceContext_Draw(ctx, 1, 0);
            ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)st);
            wait((ID3D11Asynchronous *)st, &s, sizeof s);
            static const char *cn[] = { "none", "back", "front" };
            char label[32];
            sprintf(label, "cull %s", cn[cull]);
            print_stats(label, &s);
            sprintf(label, "strip-cull-%s", cn[cull]);
            image(label, 8, 8);
            ID3D11DeviceContext_RSSetState(ctx, NULL);
            ID3D11RasterizerState_Release(rs);
        }
        ID3D11DeviceContext_GSSetShader(ctx, NULL, NULL, 0);
        ID3D11GeometryShader_Release(g);
    }

    puts("stream output of a vertex shader");
    ID3D11DeviceContext_VSSetShader(ctx, vs[VS_SO], NULL, 0);
    ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
    viewport(0, 0, W, H);
    {
        ID3D11InputLayout *lay = NULL;
        D3D11_INPUT_ELEMENT_DESC ie = { "POS", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 0, D3D11_INPUT_PER_VERTEX_DATA, 0 };
        ID3D11Device_CreateInputLayout(dev, &ie, 1, ID3D10Blob_GetBufferPointer(bl[VS_SO]), ID3D10Blob_GetBufferSize(bl[VS_SO]), &lay);
        ID3D11DeviceContext_IASetInputLayout(ctx, lay);
        static const float tri[9][4] = { { -1, -1, 0.5f, 1 }, { -1, 0, 0.5f, 1 }, { 0, -1, 0.5f, 1 }, { 0, 0, 0.5f, 1 }, { 0, 1, 0.5f, 1 }, { 1, 0, 0.5f, 1 },
            { -1, 0, 0.5f, 1 }, { -1, 1, 0.5f, 1 }, { 0, 0, 0.5f, 1 } };
        ID3D11Buffer *vb = buffer(sizeof tri, D3D11_BIND_VERTEX_BUFFER, tri);
        UINT vstride = 16, voff = 0;
        ID3D11DeviceContext_IASetVertexBuffers(ctx, 0, 1, &vb, &vstride, &voff);
        ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);

        /* positions only, rasterized too; a buffer of two triangles */
        D3D11_SO_DECLARATION_ENTRY pe = { 0, "SV_Position", 0, 0, 4, 0 };
        UINT pstride = 16;
        HRESULT hr;
        ID3D11GeometryShader *g = so_shader(bl[VS_SO], &pe, 1, &pstride, 1, 0, &hr);
        printf("  CreateGeometryShaderWithStreamOutput(vertex shader, SV_Position) %#lx\n", hr);
        ID3D11Buffer *small = so_buffer(24);
        UINT off = 0;
        ID3D11DeviceContext_SOSetTargets(ctx, 1, &small, &off);
        ID3D11DeviceContext_GSSetShader(ctx, g, NULL, 0);
        clear();
        D3D11_QUERY_DATA_SO_STATISTICS so = { 7, 7 };
        D3D11_QUERY_DATA_PIPELINE_STATISTICS s;
        BOOL over = 7;
        UINT64 v = 0;
        ID3D11Asynchronous *qs[4] = { (ID3D11Asynchronous *)sos, (ID3D11Asynchronous *)st, (ID3D11Asynchronous *)ovf, (ID3D11Asynchronous *)occ };
        for (int k = 0; k < 4; k++) ID3D11DeviceContext_Begin(ctx, qs[k]);
        ID3D11DeviceContext_Draw(ctx, 9, 0);
        for (int k = 0; k < 4; k++) ID3D11DeviceContext_End(ctx, qs[k]);
        wait(qs[0], &so, sizeof so);
        wait(qs[1], &s, sizeof s);
        wait(qs[2], &over, sizeof over);
        wait(qs[3], &v, 8);
        print_stats("3 triangles into room for 2", &s);
        printf("  written %llu needed %llu overflow %d samples %llu pixels %d\n", (unsigned long long)so.NumPrimitivesWritten, (unsigned long long)so.PrimitivesStorageNeeded, over,
            (unsigned long long)v, written());
        dump("buffer", small, 24, 4);
        ID3D11Buffer *got[4] = { 0 };
        ID3D11DeviceContext_SOGetTargets(ctx, 4, got);
        printf("  SOGetTargets: %d %d %d %d\n", got[0] == small, got[1] != NULL, got[2] != NULL, got[3] != NULL);
        for (int k = 0; k < 4; k++)
            if (got[k]) ID3D11Buffer_Release(got[k]);

        /* appending (-1) after a draw, then 0 again */
        ID3D11Buffer *big = so_buffer(64);
        off = 0;
        ID3D11DeviceContext_SOSetTargets(ctx, 1, &big, &off);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        off = (UINT)-1;
        ID3D11DeviceContext_SOSetTargets(ctx, 1, &big, &off);
        ID3D11DeviceContext_Draw(ctx, 3, 3);
        ID3D11DeviceContext_Draw(ctx, 3, 6);
        dump("draws of 3 at 0, then -1 (append) twice", big, 40, 4);
        off = 16;
        ID3D11DeviceContext_SOSetTargets(ctx, 1, &big, &off);
        ID3D11DeviceContext_Draw(ctx, 3, 6);
        dump("then offset 16", big, 20, 4);

        /* DrawAuto of what was streamed out */
        ID3D11Buffer *auto_b = so_buffer(64);
        off = 0;
        ID3D11DeviceContext_SOSetTargets(ctx, 1, &auto_b, &off);
        ID3D11DeviceContext_Draw(ctx, 6, 0);
        off = (UINT)-1;
        ID3D11DeviceContext_SOSetTargets(ctx, 1, &auto_b, &off);
        ID3D11DeviceContext_Draw(ctx, 3, 6);
        ID3D11DeviceContext_SOSetTargets(ctx, 0, NULL, NULL);
        ID3D11DeviceContext_GSSetShader(ctx, NULL, NULL, 0);
        ID3D11DeviceContext_VSSetShader(ctx, vs[VS_POS], NULL, 0);
        ID3D11DeviceContext_IASetInputLayout(ctx, pos_layout);
        ID3D11DeviceContext_IASetVertexBuffers(ctx, 0, 1, &auto_b, &pstride, &voff);
        clear();
        ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)st);
        ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)occ);
        ID3D11DeviceContext_DrawAuto(ctx);
        ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)occ);
        ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)st);
        wait((ID3D11Asynchronous *)st, &s, sizeof s);
        wait((ID3D11Asynchronous *)occ, &v, 8);
        print_stats("DrawAuto after 6 + 3 vertices", &s);
        printf("  samples %llu pixels %d\n", (unsigned long long)v, written());
        voff = 32;
        ID3D11DeviceContext_IASetVertexBuffers(ctx, 0, 1, &auto_b, &pstride, &voff);
        ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)st);
        ID3D11DeviceContext_DrawAuto(ctx);
        ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)st);
        wait((ID3D11Asynchronous *)st, &s, sizeof s);
        print_stats("DrawAuto with the vertex buffer at offset 32", &s);
        voff = 0;

        /* declarations with gaps, start components and two slots */
        ID3D11DeviceContext_VSSetShader(ctx, vs[VS_SO], NULL, 0);
        ID3D11DeviceContext_IASetInputLayout(ctx, lay);
        ID3D11DeviceContext_IASetVertexBuffers(ctx, 0, 1, &vb, &vstride, &voff);
        static const D3D11_SO_DECLARATION_ENTRY ge[] = {
            { 0, "COL", 0, 1, 2, 0 }, { 0, NULL, 0, 0, 2, 0 }, { 0, "TEX", 0, 0, 1, 0 },
            { 0, "TEX", 0, 1, 1, 1 }, { 0, "SV_Position", 0, 0, 2, 1 },
        };
        UINT gs2[2] = { 24, 16 };
        ID3D11GeometryShader *g2 = so_shader(bl[VS_SO], ge, 5, gs2, 2, D3D11_SO_NO_RASTERIZED_STREAM, &hr);
        printf("  gaps / two slots: %#lx\n", hr);
        ID3D11Buffer *b0 = so_buffer(64), *b1 = so_buffer(64);
        ID3D11Buffer *bs[2] = { b0, b1 };
        UINT offs[2] = { 0, 4 };
        ID3D11DeviceContext_SOSetTargets(ctx, 2, bs, offs);
        ID3D11DeviceContext_GSSetShader(ctx, g2, NULL, 0);
        clear();
        ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)occ);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)occ);
        wait((ID3D11Asynchronous *)occ, &v, 8);
        printf("  NO_RASTERIZED_STREAM samples %llu\n", (unsigned long long)v);
        dump("slot 0 (stride 24)", b0, 18, 6);
        dump("slot 1 (stride 16, offset 4)", b1, 14, 4);

        /* errors */
        static const struct { const char *name; D3D11_SO_DECLARATION_ENTRY e; UINT stride; UINT raster; } bad[] = {
            { "unknown semantic", { 0, "NOPE", 0, 0, 4, 0 }, 16, 0 },
            { "stride too small", { 0, "COL", 0, 0, 4, 0 }, 12, 0 },
            { "stride not a multiple of 4", { 0, "COL", 0, 0, 1, 0 }, 6, 0 },
            { "start + count over 4", { 0, "COL", 0, 2, 3, 0 }, 16, 0 },
            { "slot 4", { 0, "COL", 0, 0, 4, 4 }, 16, 0 },
            { "stream 1 of a vertex shader", { 1, "COL", 0, 0, 4, 0 }, 16, 0 },
            { "rasterized stream 1", { 0, "COL", 0, 0, 4, 0 }, 16, 1 },
            { "gap of 0 components", { 0, NULL, 0, 0, 0, 0 }, 16, 0 },
            { "a wider stride", { 0, "COL", 0, 0, 4, 0 }, 2048, 0 },
        };
        for (int i = 0; i < (int)(sizeof bad / sizeof bad[0]); i++) {
            ID3D11GeometryShader *x = so_shader(bl[VS_SO], &bad[i].e, 1, &bad[i].stride, 1, bad[i].raster, &hr);
            printf("  %s: %#lx\n", bad[i].name, hr);
            if (x) ID3D11GeometryShader_Release(x);
        }
        {
            ID3D11GeometryShader *x = NULL;
            hr = ID3D11Device_CreateGeometryShaderWithStreamOutput(dev, NULL, 0, &pe, 1, &pstride, 1, 0, NULL, &x);
            printf("  no bytecode: %#lx %d\n", hr, x != NULL);
            if (x) ID3D11GeometryShader_Release(x);
            hr = ID3D11Device_CreateGeometryShaderWithStreamOutput(dev, ID3D10Blob_GetBufferPointer(bl[VS_SO]), ID3D10Blob_GetBufferSize(bl[VS_SO]), &pe, 1, &pstride, 1, 0, NULL, NULL);
            printf("  NULL out: %#lx\n", hr);
            hr = ID3D11Device_CreateGeometryShaderWithStreamOutput(dev, ID3D10Blob_GetBufferPointer(bl[VS_SO]), ID3D10Blob_GetBufferSize(bl[VS_SO]), NULL, 0, &pstride, 1, 0, NULL, &x);
            printf("  no entries: %#lx\n", hr);
            if (x) ID3D11GeometryShader_Release(x);
        }
        ID3D11DeviceContext_SOSetTargets(ctx, 0, NULL, NULL);
        ID3D11DeviceContext_GSSetShader(ctx, NULL, NULL, 0);
    }

    puts("two streams of a geometry shader");
    {
        ID3D11DeviceContext_VSSetShader(ctx, vs[VS_ID], NULL, 0);
        ID3D11DeviceContext_IASetInputLayout(ctx, NULL);
        ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_POINTLIST);
        static const D3D11_SO_DECLARATION_ENTRY e[] = { { 0, "A", 0, 0, 1, 0 }, { 1, "A", 0, 0, 1, 1 } };
        UINT strides[2] = { 4, 4 };
        HRESULT hr;
        ID3D11GeometryShader *g = so_shader(bl[GS_TWO], e, 2, strides, 2, D3D11_SO_NO_RASTERIZED_STREAM, &hr);
        printf("  create %#lx\n", hr);
        if (g) {
            ID3D11Buffer *b0 = so_buffer(16), *b1 = so_buffer(16);
            ID3D11Buffer *bs[2] = { b0, b1 };
            UINT offs[2] = { 0, 0 };
            ID3D11DeviceContext_SOSetTargets(ctx, 2, bs, offs);
            ID3D11DeviceContext_GSSetShader(ctx, g, NULL, 0);
            ID3D11Query *s0 = query(D3D11_QUERY_SO_STATISTICS_STREAM0), *s1 = query(D3D11_QUERY_SO_STATISTICS_STREAM1);
            D3D11_QUERY_DATA_SO_STATISTICS a = { 7, 7 }, b = { 7, 7 }, all = { 7, 7 };
            ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)s0);
            ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)s1);
            ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)sos);
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)sos);
            ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)s1);
            ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)s0);
            wait((ID3D11Asynchronous *)s0, &a, sizeof a);
            wait((ID3D11Asynchronous *)s1, &b, sizeof b);
            wait((ID3D11Asynchronous *)sos, &all, sizeof all);
            printf("  stream 0 %llu %llu stream 1 %llu %llu all %llu %llu\n", (unsigned long long)a.NumPrimitivesWritten, (unsigned long long)a.PrimitivesStorageNeeded,
                (unsigned long long)b.NumPrimitivesWritten, (unsigned long long)b.PrimitivesStorageNeeded, (unsigned long long)all.NumPrimitivesWritten,
                (unsigned long long)all.PrimitivesStorageNeeded);
            dump("stream 0", b0, 8, 8);
            dump("stream 1", b1, 8, 8);
        }
    }
    return 0;
}
