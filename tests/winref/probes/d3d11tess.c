/* Direct3D 11 tessellation: hull and domain shaders run (feature level 11_0):
 *  - the domain points (SV_DomainLocation) and primitives the tessellator makes for each
 *    domain (tri, quad, isoline), partitioning (integer, pow2, fractional_odd,
 *    fractional_even) and output topology (point, line, triangle_cw, triangle_ccw), captured
 *    by stream output of the domain shader, for tess factors 1, small, fractional, 0 (the patch
 *    culled), NaN, negative, 64, past 64, past [maxtessfactor];
 *  - which control points a domain shader is given (OutputPatch, the hull shader's ids),
 *    SV_PrimitiveID of the hull and domain shaders, the patch constants it reads;
 *  - PIPELINE_STATISTICS (HSInvocations, DSInvocations) and SO_STATISTICS of each;
 *  - control points past a patch list's last whole patch;
 *  - CreateHullShader / CreateDomainShader of another stage's bytecode.
 * Large outputs print their counts and an FNV-1a hash of the captured bytes.
 * WARP or hardware on the Windows runner.
 * libs: -ld3d11 -ld3dcompiler_47
 * run: browser (sake's Direct3D 11 runs in the page: tests/winref/run-browser.mjs)
 * lines: all (thousands of lines of points at once) */
#define COBJMACROS
#define INITGUID
#include "../probe.h"
#include <d3d11.h>
#include <d3dcompiler.h>
#include <math.h>

static ID3D11Device *dev;
static ID3D11DeviceContext *ctx;

static const char *SRC =
    "cbuffer F : register(b0) { float4 e; float4 ins; };\n"
    "struct CP { uint id : ID; };\n"
    "CP vs(uint id : SV_VertexID) { CP o; o.id = id; return o; }\n"
    "struct TriPC { float e[3] : SV_TessFactor; float i : SV_InsideTessFactor; uint pid : PID; };\n"
    "struct QuadPC { float e[4] : SV_TessFactor; float i[2] : SV_InsideTessFactor; uint pid : PID; };\n"
    "struct LinePC { float e[2] : SV_TessFactor; uint pid : PID; };\n"
    "TriPC pc_tri(InputPatch<CP, 3> ip, uint pid : SV_PrimitiveID) { TriPC o; o.e[0] = e.x; o.e[1] = e.y; o.e[2] = e.z; o.i = ins.x; o.pid = pid * 100 + ip[2].id; return o; }\n"
    "QuadPC pc_quad(InputPatch<CP, 4> ip, uint pid : SV_PrimitiveID) { QuadPC o; o.e[0] = e.x; o.e[1] = e.y; o.e[2] = e.z; o.e[3] = e.w; o.i[0] = ins.x; o.i[1] = ins.y; o.pid = pid * 100 + ip[3].id; return o; }\n"
    "LinePC pc_line(InputPatch<CP, 2> ip, uint pid : SV_PrimitiveID) { LinePC o; o.e[0] = e.x; o.e[1] = e.y; o.pid = pid * 100 + ip[1].id; return o; }\n"
    "struct OP { uint id : ID; uint c : C; };\n"
    "#define HS(n, d, p, t, k, pc) [domain(d)] [partitioning(p)] [outputtopology(t)] [outputcontrolpoints(k)] [patchconstantfunc(pc)] OP n(InputPatch<CP, k> ip, uint c : SV_OutputControlPointID, uint pid : SV_PrimitiveID) { OP o; o.id = ip[c].id + pid * 1000; o.c = c; return o; }\n"
    "HS(hs_tri_int, \"tri\", \"integer\", \"triangle_cw\", 3, \"pc_tri\")\n"
    "HS(hs_tri_pow2, \"tri\", \"pow2\", \"triangle_cw\", 3, \"pc_tri\")\n"
    "HS(hs_tri_odd, \"tri\", \"fractional_odd\", \"triangle_cw\", 3, \"pc_tri\")\n"
    "HS(hs_tri_even, \"tri\", \"fractional_even\", \"triangle_cw\", 3, \"pc_tri\")\n"
    "HS(hs_tri_ccw, \"tri\", \"integer\", \"triangle_ccw\", 3, \"pc_tri\")\n"
    "HS(hs_tri_point, \"tri\", \"fractional_odd\", \"point\", 3, \"pc_tri\")\n"
    "HS(hs_quad_int, \"quad\", \"integer\", \"triangle_cw\", 4, \"pc_quad\")\n"
    "HS(hs_quad_pow2, \"quad\", \"pow2\", \"triangle_ccw\", 4, \"pc_quad\")\n"
    "HS(hs_quad_odd, \"quad\", \"fractional_odd\", \"triangle_cw\", 4, \"pc_quad\")\n"
    "HS(hs_quad_even, \"quad\", \"fractional_even\", \"triangle_cw\", 4, \"pc_quad\")\n"
    "HS(hs_quad_point, \"quad\", \"integer\", \"point\", 4, \"pc_quad\")\n"
    "HS(hs_line_int, \"isoline\", \"integer\", \"line\", 2, \"pc_line\")\n"
    "HS(hs_line_odd, \"isoline\", \"fractional_odd\", \"line\", 2, \"pc_line\")\n"
    "HS(hs_line_even, \"isoline\", \"fractional_even\", \"line\", 2, \"pc_line\")\n"
    "HS(hs_line_point, \"isoline\", \"integer\", \"point\", 2, \"pc_line\")\n"
    "[domain(\"tri\")] [partitioning(\"integer\")] [outputtopology(\"triangle_cw\")] [outputcontrolpoints(3)] [patchconstantfunc(\"pc_tri\")] [maxtessfactor(5.5)]\n"
    "OP hs_tri_max(InputPatch<CP, 3> ip, uint c : SV_OutputControlPointID) { OP o; o.id = ip[c].id; o.c = c; return o; }\n"
    "[domain(\"tri\")] [partitioning(\"integer\")] [outputtopology(\"triangle_cw\")] [outputcontrolpoints(1)] [patchconstantfunc(\"pc_tri\")]\n"
    "OP hs_tri_one(InputPatch<CP, 3> ip, uint c : SV_OutputControlPointID) { OP o; o.id = ip[0].id + ip[1].id * 10 + ip[2].id * 100; o.c = c; return o; }\n"
    "struct D { float4 l : L; uint4 b : B; };\n"
    "[domain(\"tri\")] D ds_tri(TriPC pc, float3 uvw : SV_DomainLocation, OutputPatch<OP, 3> op, uint pid : SV_PrimitiveID) { D o; o.l = float4(uvw, pc.i); o.b = uint4(op[0].id, op[1].id * 16 + op[1].c, op[2].id, pc.pid * 16 + pid); return o; }\n"
    "[domain(\"tri\")] D ds_tri_one(TriPC pc, float3 uvw : SV_DomainLocation, OutputPatch<OP, 1> op, uint pid : SV_PrimitiveID) { D o; o.l = float4(uvw, pc.e[1]); o.b = uint4(op[0].id, op[0].c, pc.pid, pid); return o; }\n"
    "[domain(\"quad\")] D ds_quad(QuadPC pc, float2 uv : SV_DomainLocation, OutputPatch<OP, 4> op, uint pid : SV_PrimitiveID) { D o; o.l = float4(uv, pc.i[1], pc.e[3]); o.b = uint4(op[0].id, op[3].id * 16 + op[3].c, pc.pid, pid); return o; }\n"
    "[domain(\"isoline\")] D ds_line(LinePC pc, float2 uv : SV_DomainLocation, OutputPatch<OP, 2> op, uint pid : SV_PrimitiveID) { D o; o.l = float4(uv, pc.e[0], pc.e[1]); o.b = uint4(op[0].id, op[1].id, pc.pid, pid); return o; }\n";

static ID3DBlob *compile(const char *entry, const char *target) {
    ID3DBlob *b = NULL, *err = NULL;
    HRESULT hr = D3DCompile(SRC, strlen(SRC), "tess", NULL, NULL, entry, target, 0, 0, &b, &err);
    if (FAILED(hr)) printf("D3DCompile %s failed %#lx %s\n", entry, hr, err ? (char *)ID3D10Blob_GetBufferPointer(err) : "");
    if (err) ID3D10Blob_Release(err);
    return b;
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

/* one captured vertex: SV_DomainLocation and a patch constant (L), ids (B) */
#define VERT 8
/* room for the most a patch makes (a quad at 64: 8192 triangles) and one more patch */
#define ROOM (2 * 8192 * 3 * VERT)

/* the captured vertices: each one's L as floats (%.9g) and B, `per` vertices to a primitive;
 * past `most` vertices only their count and an FNV-1a hash of their bytes */
static void dump(ID3D11Buffer *so, UINT64 prims, UINT per, UINT most) {
    D3D11_BUFFER_DESC bd;
    ID3D11Buffer_GetDesc(so, &bd);
    bd.Usage = D3D11_USAGE_STAGING;
    bd.BindFlags = 0;
    bd.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    bd.MiscFlags = 0;
    ID3D11Buffer *s = NULL;
    ID3D11Device_CreateBuffer(dev, &bd, NULL, &s);
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)s, (ID3D11Resource *)so);
    D3D11_MAPPED_SUBRESOURCE m;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)s, 0, D3D11_MAP_READ, 0, &m))) { puts("    Map failed"); return; }
    const UINT *p = m.pData;
    UINT64 n = prims * per;
    if (n * VERT > ROOM) n = ROOM / VERT;
    if (n <= most) {
        for (UINT64 v = 0; v < n; v++) {
            const UINT *x = p + v * VERT;
            float f[4];
            memcpy(f, x, sizeof f);
            printf("%s%.9g %.9g %.9g %.9g | %u %u %u %u", v % per == 0 ? "    " : " ; ", f[0], f[1], f[2], f[3], x[4], x[5], x[6], x[7]);
            if (v % per == per - 1) puts("");
        }
    } else {
        UINT h = 2166136261u;
        const unsigned char *b = m.pData;
        for (UINT64 i = 0; i < n * VERT * 4; i++) h = (h ^ b[i]) * 16777619u;
        printf("    %llu vertices, fnv %08x\n", (unsigned long long)n, h);
    }
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)s, 0);
    ID3D11Buffer_Release(s);
}

/* the patch constant signature (PCSG) of a compiled hull or domain shader: name, index,
 * system value, register, mask of each element */
static void pcsg(const char *entry, ID3DBlob *b) {
    const unsigned char *p = ID3D10Blob_GetBufferPointer(b);
    UINT size = (UINT)ID3D10Blob_GetBufferSize(b), n;
    memcpy(&n, p + 28, 4);
    printf("  %s:", entry);
    for (UINT i = 0; i < n; i++) {
        UINT off, count;
        memcpy(&off, p + 32 + 4 * i, 4);
        if (off + 8 > size || memcmp(p + off, "PCSG", 4)) continue;
        const unsigned char *c = p + off + 8;
        memcpy(&count, c, 4);
        for (UINT k = 0; k < count; k++) {
            UINT w[5];
            memcpy(w, c + 8 + 24 * k, sizeof w);
            printf(" %s%u sv%u r%u m%x;", (const char *)c + w[0], w[1], w[2], w[4], c[8 + 24 * k + 20]);
        }
    }
    puts("");
}

static ID3D11Query *st, *sos;
static ID3D11Buffer *cb, *so;

/* one draw of `count` control points: its statistics and what stream output caught */
static void draw(const char *label, const float *factors, UINT count, UINT per, UINT most) {
    if (factors) ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)cb, 0, NULL, factors, 0, 0);
    UINT off = 0;
    ID3D11DeviceContext_SOSetTargets(ctx, 1, &so, &off);
    D3D11_QUERY_DATA_PIPELINE_STATISTICS s;
    D3D11_QUERY_DATA_SO_STATISTICS w = { 7, 7 };
    ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)st);
    ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)sos);
    ID3D11DeviceContext_Draw(ctx, count, 0);
    ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)sos);
    ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)st);
    wait((ID3D11Asynchronous *)st, &s, sizeof s);
    wait((ID3D11Asynchronous *)sos, &w, sizeof w);
    printf("  %s", label);
    if (factors) printf(" (%g %g %g %g / %g %g)", factors[0], factors[1], factors[2], factors[3], factors[4], factors[5]);
    printf(": IA %llu %llu VS %llu HS %llu DS %llu GS %llu C %llu PS %llu SO %llu %llu\n", (unsigned long long)s.IAVertices, (unsigned long long)s.IAPrimitives,
        (unsigned long long)s.VSInvocations, (unsigned long long)s.HSInvocations, (unsigned long long)s.DSInvocations, (unsigned long long)s.GSInvocations,
        (unsigned long long)s.CInvocations, (unsigned long long)s.PSInvocations, (unsigned long long)w.NumPrimitivesWritten, (unsigned long long)w.PrimitivesStorageNeeded);
    ID3D11DeviceContext_SOSetTargets(ctx, 0, NULL, NULL);
    dump(so, w.NumPrimitivesWritten, per, most);
}

static ID3D11GeometryShader *so_of(ID3DBlob *ds) {
    static const D3D11_SO_DECLARATION_ENTRY e[] = { { 0, "L", 0, 0, 4, 0 }, { 0, "B", 0, 0, 4, 0 } };
    UINT stride = VERT * 4;
    ID3D11GeometryShader *g = NULL;
    HRESULT hr = ID3D11Device_CreateGeometryShaderWithStreamOutput(dev, ID3D10Blob_GetBufferPointer(ds), ID3D10Blob_GetBufferSize(ds), e, 2, &stride, 1, D3D11_SO_NO_RASTERIZED_STREAM, NULL, &g);
    if (FAILED(hr)) printf("  stream output of the domain shader: %#lx\n", hr);
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

    enum { VS, HS_TRI_INT, HS_TRI_POW2, HS_TRI_ODD, HS_TRI_EVEN, HS_TRI_CCW, HS_TRI_POINT, HS_QUAD_INT, HS_QUAD_POW2, HS_QUAD_ODD, HS_QUAD_EVEN, HS_QUAD_POINT,
        HS_LINE_INT, HS_LINE_ODD, HS_LINE_EVEN, HS_LINE_POINT, HS_TRI_MAX, HS_TRI_ONE, DS_TRI, DS_TRI_ONE, DS_QUAD, DS_LINE, N };
    static const char *names[N] = { "vs", "hs_tri_int", "hs_tri_pow2", "hs_tri_odd", "hs_tri_even", "hs_tri_ccw", "hs_tri_point", "hs_quad_int", "hs_quad_pow2",
        "hs_quad_odd", "hs_quad_even", "hs_quad_point", "hs_line_int", "hs_line_odd", "hs_line_even", "hs_line_point", "hs_tri_max", "hs_tri_one", "ds_tri",
        "ds_tri_one", "ds_quad", "ds_line" };
    ID3DBlob *bl[N];
    for (int i = 0; i < N; i++)
        if (!(bl[i] = compile(names[i], i == VS ? "vs_5_0" : i < DS_TRI ? "hs_5_0" : "ds_5_0"))) return 0;
    puts("patch constant signatures");
    for (int i = HS_TRI_INT; i < N; i++) pcsg(names[i], bl[i]);
    ID3D11VertexShader *vs = NULL;
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(bl[VS]), ID3D10Blob_GetBufferSize(bl[VS]), NULL, &vs);
    ID3D11HullShader *hs[N] = { 0 };
    ID3D11DomainShader *ds[N] = { 0 };
    ID3D11GeometryShader *gs[N] = { 0 };
    for (int i = HS_TRI_INT; i < DS_TRI; i++) {
        HRESULT hr = ID3D11Device_CreateHullShader(dev, ID3D10Blob_GetBufferPointer(bl[i]), ID3D10Blob_GetBufferSize(bl[i]), NULL, &hs[i]);
        if (FAILED(hr)) printf("CreateHullShader %s %#lx\n", names[i], hr);
    }
    for (int i = DS_TRI; i < N; i++) {
        HRESULT hr = ID3D11Device_CreateDomainShader(dev, ID3D10Blob_GetBufferPointer(bl[i]), ID3D10Blob_GetBufferSize(bl[i]), NULL, &ds[i]);
        if (FAILED(hr)) printf("CreateDomainShader %s %#lx\n", names[i], hr);
        gs[i] = so_of(bl[i]);
    }
    st = query(D3D11_QUERY_PIPELINE_STATISTICS);
    sos = query(D3D11_QUERY_SO_STATISTICS);
    if (!st || !sos) { puts("skip no queries"); return 0; }
    cb = buffer(32, D3D11_BIND_CONSTANT_BUFFER, NULL);
    so = buffer(ROOM * 4, D3D11_BIND_STREAM_OUTPUT, NULL);
    ID3D11DeviceContext_VSSetShader(ctx, vs, NULL, 0);
    ID3D11DeviceContext_HSSetConstantBuffers(ctx, 0, 1, &cb);
    ID3D11DeviceContext_PSSetShader(ctx, NULL, NULL, 0);

    const float nan = NAN, inf = INFINITY;
    /* edges (x y z w), insides (x y) */
    static float fs[][8] = {
        { 1, 1, 1, 1, 1, 1 },
        { 2, 3, 4, 5, 3, 2 },
        { 1, 1, 1, 1, 3, 4 },
        { 3, 3, 3, 3, 1, 1 },
        { 2.5f, 1.5f, 3.3f, 1.01f, 2.2f, 4.9f },
        { 1.99f, 2.01f, 6.5f, 7.5f, 5.5f, 6.25f },
        { 0.5f, 0.25f, 1, 1.5f, 0.1f, 0 },
        { 2, 2, 2, 2, 0, 0 },
        { 0, 2, 2, 2, 2, 2 },
        { 2, 2, 2, 0, 2, 2 },
        { -1, 2, 2, 2, 2, 2 },
        { 2, 2, 2, 2, -1, -1 },
        { 0, 0, 0, 0, 0, 0 },
        { 63.5f, 64, 63, 62.9f, 64, 63.75f },
        { 64, 64, 64, 64, 64, 64 },
        { 65, 100, 1000, 64.5f, 70, 1e30f },
    };
    enum { SMALL = 13 };
    float special[3][8] = { { nan, 2, 2, 2, 2, 2 }, { 2, 2, 2, 2, nan, nan }, { inf, 2, 2, 2, 2, 2 } };
    static const struct { const char *name; int hs, ds; D3D11_PRIMITIVE_TOPOLOGY t; UINT count, per; } c[] = {
        { "tri integer triangle_cw", HS_TRI_INT, DS_TRI, D3D11_PRIMITIVE_TOPOLOGY_3_CONTROL_POINT_PATCHLIST, 6, 3 },
        { "tri pow2 triangle_cw", HS_TRI_POW2, DS_TRI, D3D11_PRIMITIVE_TOPOLOGY_3_CONTROL_POINT_PATCHLIST, 3, 3 },
        { "tri fractional_odd triangle_cw", HS_TRI_ODD, DS_TRI, D3D11_PRIMITIVE_TOPOLOGY_3_CONTROL_POINT_PATCHLIST, 3, 3 },
        { "tri fractional_even triangle_cw", HS_TRI_EVEN, DS_TRI, D3D11_PRIMITIVE_TOPOLOGY_3_CONTROL_POINT_PATCHLIST, 3, 3 },
        { "tri integer triangle_ccw", HS_TRI_CCW, DS_TRI, D3D11_PRIMITIVE_TOPOLOGY_3_CONTROL_POINT_PATCHLIST, 3, 3 },
        { "tri fractional_odd point", HS_TRI_POINT, DS_TRI, D3D11_PRIMITIVE_TOPOLOGY_3_CONTROL_POINT_PATCHLIST, 3, 1 },
        { "quad integer triangle_cw", HS_QUAD_INT, DS_QUAD, D3D11_PRIMITIVE_TOPOLOGY_4_CONTROL_POINT_PATCHLIST, 8, 3 },
        { "quad pow2 triangle_ccw", HS_QUAD_POW2, DS_QUAD, D3D11_PRIMITIVE_TOPOLOGY_4_CONTROL_POINT_PATCHLIST, 4, 3 },
        { "quad fractional_odd triangle_cw", HS_QUAD_ODD, DS_QUAD, D3D11_PRIMITIVE_TOPOLOGY_4_CONTROL_POINT_PATCHLIST, 4, 3 },
        { "quad fractional_even triangle_cw", HS_QUAD_EVEN, DS_QUAD, D3D11_PRIMITIVE_TOPOLOGY_4_CONTROL_POINT_PATCHLIST, 4, 3 },
        { "quad integer point", HS_QUAD_POINT, DS_QUAD, D3D11_PRIMITIVE_TOPOLOGY_4_CONTROL_POINT_PATCHLIST, 4, 1 },
        { "isoline integer line", HS_LINE_INT, DS_LINE, D3D11_PRIMITIVE_TOPOLOGY_2_CONTROL_POINT_PATCHLIST, 4, 2 },
        { "isoline fractional_odd line", HS_LINE_ODD, DS_LINE, D3D11_PRIMITIVE_TOPOLOGY_2_CONTROL_POINT_PATCHLIST, 2, 2 },
        { "isoline fractional_even line", HS_LINE_EVEN, DS_LINE, D3D11_PRIMITIVE_TOPOLOGY_2_CONTROL_POINT_PATCHLIST, 2, 2 },
        { "isoline integer point", HS_LINE_POINT, DS_LINE, D3D11_PRIMITIVE_TOPOLOGY_2_CONTROL_POINT_PATCHLIST, 2, 1 },
        { "tri maxtessfactor(5.5)", HS_TRI_MAX, DS_TRI, D3D11_PRIMITIVE_TOPOLOGY_3_CONTROL_POINT_PATCHLIST, 3, 3 },
        { "tri, 1 output control point", HS_TRI_ONE, DS_TRI_ONE, D3D11_PRIMITIVE_TOPOLOGY_3_CONTROL_POINT_PATCHLIST, 6, 3 },
    };
    for (int i = 0; i < (int)(sizeof c / sizeof c[0]); i++) {
        printf("%s\n", c[i].name);
        ID3D11DeviceContext_IASetPrimitiveTopology(ctx, c[i].t);
        ID3D11DeviceContext_HSSetShader(ctx, hs[c[i].hs], NULL, 0);
        ID3D11DeviceContext_DSSetShader(ctx, ds[c[i].ds], NULL, 0);
        ID3D11DeviceContext_GSSetShader(ctx, gs[c[i].ds], NULL, 0);
        for (int f = 0; f < (int)(sizeof fs / sizeof fs[0]); f++) draw("", fs[f], c[i].count, c[i].per, f < SMALL ? 400 : 0);
        for (int f = 0; f < 3; f++) draw("", special[f], c[i].count, c[i].per, 400);
    }

    /* (a patch list with stages that do not go together, or with a topology that is not one
     * with hull and domain shaders bound, is an error Direct3D leaves undefined: WARP crashes
     * in the first such draw, hardware draws something of its own: not asked here) */
    puts("control points past the last whole patch");
    {
        float two[8] = { 2, 2, 2, 2, 2, 2 };
        ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)cb, 0, NULL, two, 0, 0);
        ID3D11DeviceContext_HSSetShader(ctx, hs[HS_TRI_INT], NULL, 0);
        ID3D11DeviceContext_DSSetShader(ctx, ds[DS_TRI], NULL, 0);
        ID3D11DeviceContext_GSSetShader(ctx, gs[DS_TRI], NULL, 0);
        ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_3_CONTROL_POINT_PATCHLIST);
        draw("5 control points of a 3 control point patch list", NULL, 5, 3, 100);
        ID3D11DeviceContext_HSSetShader(ctx, NULL, NULL, 0);
        ID3D11DeviceContext_DSSetShader(ctx, NULL, NULL, 0);
        ID3D11DeviceContext_GSSetShader(ctx, NULL, NULL, 0);
    }

    puts("creating shaders of another stage's bytecode");
    {
        ID3D11HullShader *h = NULL;
        ID3D11DomainShader *d = NULL;
        HRESULT hr = ID3D11Device_CreateHullShader(dev, ID3D10Blob_GetBufferPointer(bl[DS_TRI]), ID3D10Blob_GetBufferSize(bl[DS_TRI]), NULL, &h);
        printf("  CreateHullShader of a domain shader: %#lx %d\n", hr, h != NULL);
        hr = ID3D11Device_CreateHullShader(dev, ID3D10Blob_GetBufferPointer(bl[VS]), ID3D10Blob_GetBufferSize(bl[VS]), NULL, &h);
        printf("  CreateHullShader of a vertex shader: %#lx %d\n", hr, h != NULL);
        hr = ID3D11Device_CreateDomainShader(dev, ID3D10Blob_GetBufferPointer(bl[HS_TRI_INT]), ID3D10Blob_GetBufferSize(bl[HS_TRI_INT]), NULL, &d);
        printf("  CreateDomainShader of a hull shader: %#lx %d\n", hr, d != NULL);
        ID3D11GeometryShader *g = NULL;
        static const D3D11_SO_DECLARATION_ENTRY e = { 0, "L", 0, 0, 4, 0 };
        UINT stride = 16;
        hr = ID3D11Device_CreateGeometryShaderWithStreamOutput(dev, ID3D10Blob_GetBufferPointer(bl[HS_TRI_INT]), ID3D10Blob_GetBufferSize(bl[HS_TRI_INT]), &e, 1, &stride, 1, D3D11_SO_NO_RASTERIZED_STREAM, NULL, &g);
        printf("  stream output of a hull shader: %#lx %d\n", hr, g != NULL);
    }
    return 0;
}
