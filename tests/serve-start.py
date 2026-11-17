#!/usr/bin/env python3
"""masu serve answers before it has compressed the web app (serve.rs precompress): zstd -19
over a web app with several sake builds took longer than tests/winetest/run-browser.mjs waits
for the server (winetest-gpu, run 42445713214). A stale .zst is never served: it is removed
before the server listens; the fresh one appears later and is then sent zstd-encoded.

And it answers while its bucket does not, and puts nothing in it: on worker1 (2026-11-17)
0.4.1 put the store's own trees (sake's C:\\, the benchmark's programs) in the bucket before it
listened, and through a slow uplink that kept it off its port for a quarter of an hour after
/api/update restarted it (tailscale serve answered 502). A release's page reads those trees
from the release's GitHub Pages now (masu release-trees).

    tests/serve-start.py [path to masu]
"""
import http.client, os, random, socket, subprocess, sys, tempfile, threading, time

BIN = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/masu')


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def get(port, path, zstd):
    c = http.client.HTTPConnection('127.0.0.1', port, timeout=30)
    c.request('GET', path, headers={'Accept-Encoding': 'zstd'} if zstd else {})
    r = c.getresponse()
    return r.status, r.getheader('content-encoding'), r.read()


def fail(what):
    raise SystemExit(f'serve-start: {what}')


with tempfile.TemporaryDirectory() as home:
    web = os.path.join(home, 'web')
    os.makedirs(web)
    with open(os.path.join(web, 'play.html'), 'w') as f:
        f.write('<!doctype html><title>play</title>' + 'x' * 2000)
    # ~18 MiB of script that compresses (words drawn at random): seconds at level 19
    rnd = random.Random(7)
    words = [f'w{i}' for i in range(5000)]
    with open(os.path.join(web, 'big.js'), 'w') as f:
        for _ in range(16):
            f.write(' '.join(rnd.choice(words) for _ in range(200000)) + '\n')
    big = open(os.path.join(web, 'big.js'), 'rb').read()
    # a stale .zst from before (older than its source) must not be sent
    stale = os.path.join(web, 'play.html.zst')
    with open(stale, 'wb') as f:
        f.write(b'stale')
    os.utime(stale, (time.time() - 100, time.time() - 100))

    port = free_port()
    env = dict(os.environ, HOME=home, XDG_RUNTIME_DIR='', XDG_CONFIG_HOME=f'{home}/.config')
    p = subprocess.Popen([BIN, 'serve', '--foreground', '--listen', f'127.0.0.1:{port}', '--no-prerun', '--web', web,
                          '--store', os.path.join(home, 'store'), '--dev-bucket', os.path.join(home, 'bucket')], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        t0 = time.time()
        for _ in range(300):
            try:
                st, enc, body = get(port, '/play.html', True)
                break
            except OSError:
                time.sleep(0.05)
        else:
            fail('no answer in 15 s')
        took = time.time() - t0
        if st != 200 or body == b'stale' or (enc == 'zstd' and body == b'stale'):
            fail(f'play.html: {st} {enc} {body[:20]!r}')
        print(f'ok   answers in {took:.1f} s, no stale .zst')
        st, enc, body = get(port, '/big.js', True)
        if os.path.exists(os.path.join(web, 'big.js.zst')):
            fail('big.js.zst was written before the server answered')
        if st != 200 or enc is not None or body != big:
            fail(f'big.js before its .zst: {st} {enc} {len(body)}')
        else:
            print('ok   big.js sent as it is before its .zst exists')
        for _ in range(3000):
            if os.path.exists(os.path.join(web, 'big.js.zst')):
                break
            time.sleep(0.1)
        else:
            fail('big.js.zst not written in 300 s')
        st, enc, body = get(port, '/big.js', True)
        if st != 200 or enc != 'zstd' or len(body) >= len(big):
            fail(f'big.js after: {st} {enc} {len(body)}')
        print(f'ok   then zstd-encoded ({len(body)} of {len(big)} bytes)')
    finally:
        p.kill()


def stalled_bucket():
    """A bucket that takes connections and never answers: (its port, connections so far)."""
    s = socket.socket()
    s.bind(('127.0.0.1', 0))
    s.listen(64)
    held = []

    def take():
        while True:
            c, _ = s.accept()
            held.append(c)

    threading.Thread(target=take, daemon=True).start()
    return s.getsockname()[1], held


with tempfile.TemporaryDirectory() as home:
    bport, held = stalled_bucket()
    with open(os.path.join(home, 'storage.toml'), 'w') as f:
        f.write(f'bucket = "stalled"\nregion = "us-east-1"\nendpoint = "http://127.0.0.1:{bport}"\n'
                f'active_key = "k"\ncache_gib = 0\n[keys]\nk = "{"ab" * 32}"\n')
    # the release's benchmark programs: made into the store's tree "bench", which goes up
    bench = os.path.join(home, '.local', 'share', 'masu', 'bench')
    os.makedirs(bench)
    for name in ['bench.exe', 'cpu.exe']:
        with open(os.path.join(bench, name), 'wb') as f:
            f.write(os.urandom(300_000))
    with open(os.path.join(bench, 'VERSION'), 'w') as f:
        f.write('test\n')
    port = free_port()
    env = dict(os.environ, HOME=home, XDG_RUNTIME_DIR='', XDG_CONFIG_HOME=f'{home}/.config',
               XDG_DATA_HOME=f'{home}/.local/share', MASU_STORAGE_CONFIG=os.path.join(home, 'storage.toml'),
               AWS_ACCESS_KEY_ID='t', AWS_SECRET_ACCESS_KEY='t')
    log = open(os.path.join(home, 'serve.log'), 'w')
    web = os.path.join(home, 'web')
    os.makedirs(web)
    p = subprocess.Popen([BIN, 'serve', '--foreground', '--listen', f'127.0.0.1:{port}', '--no-prerun', '--web', web,
                          '--store', os.path.join(home, 'store')], env=env, stdout=log, stderr=log)
    try:
        for _ in range(300):
            try:
                c = http.client.HTTPConnection('127.0.0.1', port, timeout=10)
                c.request('GET', '/version')
                r = c.getresponse()
                st, body = r.status, r.read()
                break
            except OSError:
                time.sleep(0.05)
        else:
            fail('no answer in 15 s while the bucket does not answer: ' + open(log.name).read()[-2000:])
        if st != 200 or b'version' not in body:
            fail(f'/version: {st} {body[:100]!r}')
        if not os.path.exists(os.path.join(home, 'store', 'trees', 'bench.tsv')):
            fail('no tree "bench" made: nothing to push')
        # (the release's GitHub Pages has them: nothing of the store goes up to a real bucket)
        time.sleep(3)
        if held:
            fail(f'{len(held)} connections to the bucket: the store\'s own trees went up')
        print('ok   answers at once, and puts nothing in a bucket that never answers')
    finally:
        p.kill()
