NAME = $(shell cargo metadata --no-deps --format-version 1 --manifest-path teleterm/Cargo.toml | jq -r '.name')
VERSION = $(shell cargo metadata --no-deps --format-version 1 --manifest-path teleterm/Cargo.toml | jq -r '.version')

INTERACTIVE_SUBCOMMANDS = stream watch record play
NONINTERACTIVE_SUBCOMMANDS = server web
SUBCOMMANDS = $(INTERACTIVE_SUBCOMMANDS) $(NONINTERACTIVE_SUBCOMMANDS)

DEB_PACKAGE = $(NAME)_$(VERSION)_amd64.deb
ARCH_PACKAGE = $(NAME)-$(VERSION)-1-x86_64.pkg.tar.xz

all:
	@cargo build
.PHONY: all

release:
	@cargo build --release
.PHONY: release

test:
	@cargo test
.PHONY: test

$(SUBCOMMANDS):
	@cargo run $@
.PHONY: $(SUBCOMMANDS)

$(NONINTERACTIVE_SUBCOMMANDS:%=d%):
	@RUST_LOG=tt=debug cargo run $$(echo $@ | sed 's/^d//')
.PHONY: $(NONINTERACTIVE_SUBCOMMANDS:%=d%)

$(INTERACTIVE_SUBCOMMANDS:%=d%):
	@echo "logging to $$(echo $@ | sed 's/^d//').log"
	@RUST_LOG=tt=debug cargo run $$(echo $@ | sed 's/^d//') 2>>$$(echo $@ | sed 's/^d//').log
.PHONY: $(INTERACTIVE_SUBCOMMANDS:%=d%)

$(SUBCOMMANDS:%=r%):
	@cargo run --release $$(echo $@ | sed 's/^r//')
.PHONY: $(SUBCOMMANDS:%=r%)

clean:
	@rm -rf *.log pkg
.PHONY: clean

cleanall: clean
	@cargo clean
.PHONY: cleanall

package: pkg/$(DEB_PACKAGE) pkg/$(ARCH_PACKAGE)
.PHONY: package

pkg:
	@mkdir pkg

pkg/$(DEB_PACKAGE): | pkg
	@cargo deb && mv target/debian/$(DEB_PACKAGE) pkg

pkg/$(DEB_PACKAGE).minisig: pkg/$(DEB_PACKAGE)
	@minisign -Sm pkg/$(DEB_PACKAGE)

pkg/$(ARCH_PACKAGE): package/arch/PKGBUILD | pkg
	@cd package/arch && makepkg -c && mv $(ARCH_PACKAGE) ../../pkg

pkg/$(ARCH_PACKAGE).minisig: pkg/$(ARCH_PACKAGE)
	@minisign -Sm pkg/$(ARCH_PACKAGE)

release-dir-deb:
	@ssh tozt.net mkdir -p releases/teleterm/deb
.PHONY: release-dir-deb

publish: publish-crates-io publish-git-tags publish-deb publish-arch
.PHONY: publish

publish-crates-io: test
	@cargo publish
.PHONY: publish-crates-io

publish-git-tags: test
	@git tag $(VERSION)
	@git push --tags
.PHONY: publish-git-tags

publish-deb: test pkg/$(DEB_PACKAGE) pkg/$(DEB_PACKAGE).minisig release-dir-deb
	@scp pkg/$(DEB_PACKAGE) pkg/$(DEB_PACKAGE).minisig tozt.net:releases/teleterm/deb
.PHONY: publish-deb

release-dir-arch:
	@ssh tozt.net mkdir -p releases/teleterm/arch
.PHONY: release-dir-arch

publish-arch: test pkg/$(ARCH_PACKAGE) pkg/$(ARCH_PACKAGE).minisig release-dir-arch
	@scp pkg/$(ARCH_PACKAGE) pkg/$(ARCH_PACKAGE).minisig tozt.net:releases/teleterm/arch
.PHONY: publish-arch

install-arch: pkg/$(ARCH_PACKAGE)
	@sudo pacman -U pkg/$(ARCH_PACKAGE)
.PHONY: install-arch

wasm: teleterm/static/teleterm_web.js teleterm/static/teleterm_web_bg.wasm
.PHONY: wasm

web rweb dweb: wasm

teleterm/static/teleterm_web_bg.wasm: target/wasm/teleterm_web_bg_opt.wasm
	@cp -f $< $@

teleterm/static/teleterm_web.js: target/wasm/teleterm_web.js
	@cp -f $< $@

target/wasm/%_opt.wasm: target/wasm/%.wasm
	@wasm-opt -Oz $< -o $@

target/wasm/teleterm_web.js target/wasm/teleterm_web_bg.wasm: teleterm-web/Cargo.toml teleterm-web/src/*.rs
	@wasm-pack build --no-typescript --target web --out-dir ../target/wasm teleterm-web
