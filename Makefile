APP       := Reze
BUNDLE_ID := com.huiyunlee.reze
BUNDLE    := src-tauri/target/release/bundle/macos/$(APP).app

.PHONY: all build install update stop reset-permissions launch

all: stop reset-permissions build install launch

build:
	bun install
	bun run tauri build

install:
	rm -rf /Applications/$(APP).app
	cp -R $(BUNDLE) /Applications/$(APP).app

update: all

stop:
	-pkill -x $(APP)

# macOS ties the Accessibility grant to the binary's signature, so every
# ad-hoc rebuild invalidates it while the entry keeps looking ticked in
# System Settings — which also caches the TCC table, hence the quit first.
reset-permissions:
	-osascript -e 'quit app "System Settings"'
	-tccutil reset Accessibility $(BUNDLE_ID)
	rm -rf /Applications/$(APP).app

launch:
	open /Applications/$(APP).app
