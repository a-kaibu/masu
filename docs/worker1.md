# worker1 で masu を動かす

masu のサーバーは worker1 (kota-server-worker-1、Ubuntu 26.04、tailnet 100.90.219.50、ユーザー kota) で
動かします (2026-11-15 に worker2 から移しました)。808 PC から `ssh worker1` で入れます。
鍵まわりは自動承認が止めるので、ここの手順は手で打ちます。

構成:

* バイナリ `~/.local/bin/masu` (install.sh)、フォント `~/.local/share/masu/fonts`
* systemd ユーザーユニット `~/.config/systemd/user/masu.service` (リポジトリの `deploy/masu.service`)。
  worker1 は Linger=yes なので、ログインしていなくても動きます。
* `~/.config/masu/env`: バケットの鍵と `MASU_STORAGE_CONFIG` (下)。
* `~/.config/masu/storage.toml`、`~/.config/masu/token`
* ストア `~/.local/share/masu/store`、ログ `~/.local/state/masu/logs`、待ち受け 127.0.0.1:8656 (ゲームは 8657)
* tailscale serve で https 8656 と 8657 を tailnet に出します。

## 初回 (worker2 から移す)

808 PC から:

```sh
# 1. masu を入れて、ユニットを置く
ssh worker1 'curl -fsSL https://raw.githubusercontent.com/a-kaibu/masu/main/install.sh | sh'
ssh worker1 'mkdir -p ~/.config/systemd/user ~/.config/masu && curl -fsSL -o ~/.config/systemd/user/masu.service https://raw.githubusercontent.com/a-kaibu/masu/main/deploy/masu.service'

# 2. worker2 の ippai を止めて、設定とデータを worker1 へ (ストアは 370 MB ほど)
ssh worker2 'systemctl --user disable --now ippai.service'
ssh worker2 'tar -C ~/.config/ippai -cf - storage.toml token' | ssh worker1 'tar -C ~/.config/masu -xf - && chmod 600 ~/.config/masu/*'
ssh worker2 'tar -C ~/.local/share/ippai -cf - .' | ssh worker1 'mkdir -p ~/.local/share/masu && tar -C ~/.local/share/masu -xf -'
```

3. worker1 で `~/.config/masu/env` を作ります (`chmod 600`)。値は worker2 の `~/.aws/credentials` の `[hf]`:

```sh
AWS_ACCESS_KEY_ID=...
AWS_SECRET_ACCESS_KEY=...
MASU_STORAGE_CONFIG=/home/kota/.config/masu/storage.toml
```

```sh
# 4. 起動して公開する
ssh worker1 'systemctl --user daemon-reload && systemctl --user enable --now masu.service'
ssh worker1 'curl -s localhost:8656/version'
ssh worker1 'tailscale serve --bg --https 8656 http://127.0.0.1:8656 && tailscale serve --bg --https 8657 http://127.0.0.1:8657'
```

ブラウザからは `https://kota-server-worker-1.<tailnet>.ts.net:8656/` で開きます。

古いストアを masu が読めなければ、`~/.local/share/masu/store` を消して空で起動します
(ゲームはバケットにあり、一覧はバケットのカタログから戻ります)。

## 更新

設定サイドバーの「更新」(`/api/update`)、または:

```sh
ssh worker1 'curl -fsSL https://raw.githubusercontent.com/a-kaibu/masu/main/install.sh | sh && systemctl --user restart masu.service'
```

ログは `ssh worker1 'journalctl --user -u masu -f'` と `~/.local/state/masu/logs/`。
