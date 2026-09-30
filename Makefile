APP       := Reze
BUNDLE_ID := com.huiyunlee.reze
UNAME     := $(shell uname -s)

.PHONY: all build install update stop reset-permissions launch

ifeq ($(UNAME),Darwin)

BUNDLE    := src-tauri/target/release/bundle/macos/$(APP).app

all: stop reset-permissions build install launch

build:
	bun install
	bun run tauri build

install:
	rm -rf /Applications/$(APP).app
	cp -R $(BUNDLE) /Applications/$(APP).app

update: all

# The executable inside the bundle is `reze`, which is the name the process
# actually has — `pkill -x Reze` matches nothing and silently leaves the old
# copy running against a bundle we are about to replace.
stop:
	-pkill -x reze

# macOS ties the Accessibility grant to the binary's signature, so every
# ad-hoc rebuild invalidates it while the entry keeps looking ticked in
# System Settings — which also caches the TCC table, hence the quit first.
reset-permissions:
	-osascript -e 'quit app "System Settings"'
	-tccutil reset Accessibility $(BUNDLE_ID)
	rm -rf /Applications/$(APP).app

launch:
	open /Applications/$(APP).app

else

# Linux (X11). Everything goes under ~/.local, so no step needs root: the
# binary on PATH, a desktop entry and icons so it shows in the app grid.
# The frontend is embedded in the binary, so the binary is the whole app —
# no bundle is built (`--no-bundle`); a .deb or AppImage would only wrap it.
PREFIX  := $(HOME)/.local
BIN     := $(PREFIX)/bin/reze
APPS    := $(PREFIX)/share/applications
ICONS   := $(PREFIX)/share/icons/hicolor

all: stop build install launch

build:
	bun install
	bun run tauri build --no-bundle

install:
	install -Dm755 src-tauri/target/release/reze $(BIN)
	install -Dm644 src-tauri/icons/32x32.png $(ICONS)/32x32/apps/reze.png
	install -Dm644 src-tauri/icons/64x64.png $(ICONS)/64x64/apps/reze.png
	install -Dm644 src-tauri/icons/128x128.png $(ICONS)/128x128/apps/reze.png
	install -Dm644 src-tauri/icons/128x128@2x.png $(ICONS)/256x256/apps/reze.png
	install -Dm644 src-tauri/icons/icon.png $(ICONS)/512x512/apps/reze.png
	mkdir -p $(APPS)
	sed 's|@BIN@|$(BIN)|' assets/reze.desktop > $(APPS)/reze.desktop
	-update-desktop-database -q $(APPS)
	-gtk-update-icon-cache -q -t $(ICONS)

update: all

stop:
	-pkill -x reze

# No permission to reset: X11 lets any client synthesize input.
reset-permissions:

# Detached, so it outlives the terminal `make` ran in. Skipped over plain SSH,
# where there is no display to put a tray icon on.
launch:
	@if [ -n "$$DISPLAY" ]; then \
		setsid -f $(BIN) >/dev/null 2>&1 </dev/null; \
		echo "Reze is running — look for the bomb in the top bar."; \
	else \
		echo "No display here; start Reze from the app grid."; \
	fi

endif
