NAME = $(shell cargo read-manifest | jq -r '.name')
VERSION = $(shell cargo read-manifest | jq -r '.version')

INTERACTIVE_SUBCOMMANDS = stream watch record play
NONINTERACTIVE_SUBCOMMANDS = server
SUBCOMMANDS = $(INTERACTIVE_SUBCOMMANDS) $(NONINTERACTIVE_SUBCOMMANDS)

all:
	@cargo build
.PHONY: all

release:
	@cargo build --release
.PHONY: release

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

package: pkg-deb pkg-arch
.PHONY: package

pkg:
	@mkdir pkg

pkg-deb: pkg
	@cargo deb && mv target/debian/$(NAME)_$(VERSION)_amd64.deb pkg
.PHONY: pkg-deb

pkg-arch: pkg package/arch/PKGBUILD
	@cd package/arch && makepkg -c && mv $(NAME)-$(VERSION)-1-x86_64.pkg.tar.xz ../../pkg
.PHONY: pkg-arch
