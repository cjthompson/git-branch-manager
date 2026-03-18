Output docs/screenshots/branch-list.gif

Set Width 1400
Set Height 600
Set FontSize 14
Set Theme "TokyoNight"

# Run setup silently before visible recording begins
Hide
Type "bash /Users/chris/dev/personal/git-branch-manager/docs/demo-setup.sh"
Enter
Sleep 5s
Type "cd /tmp/gbm-demo"
Enter
Show

# Launch the app
Type "/Users/chris/dev/personal/git-branch-manager/target/release/git-branch-manager"
Enter

# Wait for branch list to load and squash detection to complete
Sleep 5s

Screenshot docs/screenshots/branch-list.png

# Show help overlay
Type "?"
Sleep 300ms
Screenshot docs/screenshots/help.png

# Close help, select merged branches
Type "?"
Sleep 200ms
Type "m"
Sleep 400ms
Screenshot docs/screenshots/select-merged.png

Type "q"
