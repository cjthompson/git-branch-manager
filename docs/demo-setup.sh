#!/usr/bin/env bash
# Creates a demo git repository with realistic branches for screenshots
set -e

DEMO_DIR="/tmp/gbm-demo"

# Clean up and recreate
rm -rf "$DEMO_DIR"
mkdir -p "$DEMO_DIR"
cd "$DEMO_DIR"

git init -q
git config user.email "demo@example.com"
git config user.name "Demo User"
git config commit.gpgSign false
git checkout -q -b main

# Initial commit on main
echo "# My App" > README.md
git add README.md
git commit -q -m "Initial commit"

# Helper: squash-merge a branch (simulates GitHub's squash-and-merge)
# Uses two commits on the branch so the squash commit is distinct from the branch tip.
squash_merge() {
    local branch=$1 file=$2 content=$3 msg=$4
    git checkout -q -b "$branch"
    # First commit (WIP)
    echo "# wip" > "$file"
    git add "$file"
    git commit -q -m "WIP: $msg"
    # Second commit (final state)
    echo "$content" > "$file"
    git add "$file"
    git commit -q -m "$msg"
    git checkout -q main
    git merge -q --squash "$branch" 2>/dev/null
    # Squash commit message differs from the branch tip's message
    git commit -q -m "$msg (#$(git rev-list --count HEAD))"
}

# Helper: regular merge
regular_merge() {
    local branch=$1 file=$2 content=$3 msg=$4
    git checkout -q -b "$branch"
    echo "$content" > "$file"
    git add "$file"
    git commit -q -m "$msg"
    git checkout -q main
    git merge -q "$branch" -m "Merge $branch"
}

# Squash-merged branches (core feature: these are detected by the tool)
squash_merge "feat/user-authentication"  "auth.py"         "import jwt"            "Add user authentication"
squash_merge "feat/payment-integration"  "payment.py"      "import stripe"         "Add payment integration"
squash_merge "chore/update-deps"         "requirements.txt" "requests==2.31.0"     "Update dependencies"
squash_merge "fix/memory-leak"           "memory.py"        "del cache"            "Fix memory leak in cache"
squash_merge "fix/null-pointer"          "utils.py"         "if x is None: return" "Fix null pointer in utils"

# Regularly merged branches
regular_merge "fix/login-redirect"       "login.py"   "redirect('/')"          "Fix login redirect"
regular_merge "hotfix/csrf-vuln"         "csrf.py"    "token = generate_csrf()" "Fix CSRF vulnerability"

# Unmerged branches (work in progress)
git checkout -q -b "feature/dark-mode"
echo "body { background: #1a1a1a; }" > dark.css
git add dark.css
git commit -q -m "WIP: Add dark mode support"
git checkout -q main

git checkout -q -b "feature/export-api"
echo "def export_csv(): pass" > export.py
git add export.py
git commit -q -m "Add CSV export endpoint"
git checkout -q main

git checkout -q -b "release/v2.1.0"
echo "version = '2.1.0'" > version.py
git add version.py
git commit -q -m "Bump version to 2.1.0"
git checkout -q main

echo "Demo repo created at $DEMO_DIR"
