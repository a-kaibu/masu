#!/bin/bash
# Runs shellcheck (warnings and errors) over every shell script in the repository, and actionlint over
# the workflows (their run: blocks through shellcheck too). CI: the test job; locally:
#   PATH=$(tools/ci/lint-tools.sh):$PATH tools/ci/shell-lint.sh
set -euo pipefail
cd "$(dirname "$0")/../.."
PATH=$HOME/.cache/masu-lint/bin:$PATH
git ls-files -z '*.sh' ':!vendor/' | xargs -0 shellcheck -S warning
# (background: and wait: steps, which actionlint 1.7.12 does not know yet)
actionlint -shellcheck="shellcheck -S warning" \
  -ignore 'unexpected key "(background|wait)" for ' \
  -ignore 'step must run script with "run" section or run action with "uses" section'
echo "shell-lint: ok"
