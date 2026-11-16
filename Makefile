APP       := Reze
BUNDLE_ID := com.huiyunlee.reze
UNAME     := $(shell uname -s)

.PHONY: all build notarize install update stop reset-permissions launch signing

ifeq ($(UNAME),Darwin)

BUNDLE    := src-tauri/target/release/bundle/macos/$(APP).app
IMAGES    := src-tauri/target/release/bundle/dmg
INSTALLED := /Applications/$(APP).app

# The Developer ID certificate to sign with, if the keychain has one.
#
# macOS ties the Accessibility grant to who signed the app. Signed with a
# certificate, that is the same answer for every build, so the grant is given
# once and kept. With none, a build is signed by nobody: its identity is the
# hash of that one binary, and every rebuild is a stranger asking again.
IDENTITY ?= $(shell security find-identity -v -p codesigning 2>/dev/null | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)

# The keychain profile `xcrun notarytool store-credentials` saved an Apple ID
# under. Only its name is here; the password never leaves the keychain.
NOTARY ?= reze-notary

all: stop build notarize install launch

# Tauri signs the app and the disk image itself when it is told who as, with
# the hardened runtime notarization requires.
TAURI := $(if $(IDENTITY),APPLE_SIGNING_IDENTITY="$(IDENTITY)") bun run tauri build

# The disk image is laid out by scripting Finder, which macOS lets a terminal
# do once it has been allowed to — and which a build started from anything
# else (ssh, CI, an agent) times out waiting for. The app needs none of that,
# so a failed image is not a failed build: it is built again without one.
build:
	bun install
	$(TAURI) || { \
		echo "No disk image this time; building the app alone."; \
		rm -f $(IMAGES)/$(APP)_*.dmg; \
		$(TAURI) --bundles app; \
	}

# Have Apple check the build, and attach its answer to the disk image and the
# app. Keeping permissions does not need this — the signature does that — but
# without it Gatekeeper refuses a copy taken to another Mac.
#
# The disk image is what is submitted when there is one: that covers the app
# inside it in one trip. A rejection stops here, before anything is installed.
notarize:
ifeq ($(IDENTITY),)
	@echo "No Developer ID certificate in the keychain: an ad-hoc build, nothing to notarize."
else
	@if ! xcrun notarytool history --keychain-profile "$(NOTARY)" >/dev/null 2>&1; then \
		echo "Signed but not notarized: no usable '$(NOTARY)' keychain profile. See README, Signing."; \
		exit 0; \
	fi; \
	image="$$(ls -t $(IMAGES)/$(APP)_*.dmg 2>/dev/null | head -1)"; \
	upload="$$image"; \
	if [ -z "$$image" ]; then \
		upload="$$(mktemp -d)/$(APP).zip"; \
		ditto -c -k --keepParent "$(BUNDLE)" "$$upload"; \
	fi; \
	xcrun notarytool submit "$$upload" --keychain-profile "$(NOTARY)" --wait \
		| tee /dev/stderr | grep -q "status: Accepted" || { \
		echo "Apple did not accept it. 'xcrun notarytool log <id> --keychain-profile $(NOTARY)' says why."; \
		exit 1; \
	}; \
	{ [ -z "$$image" ] || xcrun stapler staple "$$image"; } && xcrun stapler staple "$(BUNDLE)"
endif

# The grant is only as stable as the signature under it, so it is cleared
# whenever an install changes who signed the app — ad-hoc to signed, one team
# to another — and on every ad-hoc install, where each build is its own
# signer. Between two builds signed by the same certificate it is left alone,
# which is the whole point of signing them.
install:
	@old="$$(codesign -dv "$(INSTALLED)" 2>&1 | sed -n 's/^TeamIdentifier=//p')"; \
	new="$$(codesign -dv "$(BUNDLE)" 2>&1 | sed -n 's/^TeamIdentifier=//p')"; \
	case "$$new" in \
		""|"not set") $(MAKE) reset-permissions ;; \
		*) [ "$$old" = "$$new" ] || $(MAKE) reset-permissions ;; \
	esac
	rm -rf "$(INSTALLED)"
	cp -R "$(BUNDLE)" "$(INSTALLED)"

update: all

# The executable inside the bundle is `reze`, which is the name the process
# actually has — `pkill -x Reze` matches nothing and silently leaves the old
# copy running against a bundle we are about to replace.
stop:
	-pkill -x reze

# Forget every permission macOS holds for Reze, so the next launch asks afresh.
# A grant given to an earlier signature keeps looking ticked in System Settings
# while doing nothing — which also caches the list, hence the quit first.
reset-permissions:
	-osascript -e 'quit app "System Settings"'
	-tccutil reset All $(BUNDLE_ID)

launch:
	open "$(INSTALLED)"

# Say how the next build will be signed, and what is missing if anything is.
signing:
ifeq ($(IDENTITY),)
	@echo "Signing:    ad-hoc. No Developer ID Application certificate in the keychain,"
	@echo "            so macOS will ask for Accessibility again after every build."
else
	@echo "Signing:    $(IDENTITY)"
	@if xcrun notarytool history --keychain-profile "$(NOTARY)" >/dev/null 2>&1; then \
		echo "Notarizing: with the '$(NOTARY)' keychain profile"; \
	else \
		echo "Notarizing: no. Nothing usable is stored as '$(NOTARY)' — see README, Signing."; \
	fi
endif
	@echo "Installed:  $$(codesign -dv "$(INSTALLED)" 2>&1 | sed -n 's/^TeamIdentifier=//p' | sed 's/^not set$$/ad-hoc/;s/^$$/nothing/')"

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
