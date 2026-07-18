# Homebrew formula for Krusty
# To use: brew install BurgessTG/tap/krusty
#
# Release CI renders this template and attaches krusty.rb to the GitHub Release.
# Publishing Formula/krusty.rb to BurgessTG/homebrew-tap is a separate maintainer
# step unless an explicitly configured, repository-scoped token enables it.

class Krusty < Formula
  desc "Terminal-based AI coding assistant powered by Claude"
  homepage "https://github.com/honeycomb-Technologies/Krusty"
  version "VERSION_PLACEHOLDER"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/honeycomb-Technologies/Krusty/releases/download/vVERSION_PLACEHOLDER/krusty-aarch64-apple-darwin.tar.gz"
      sha256 "SHA256_MACOS_ARM64"
    else
      url "https://github.com/honeycomb-Technologies/Krusty/releases/download/vVERSION_PLACEHOLDER/krusty-x86_64-apple-darwin.tar.gz"
      sha256 "SHA256_MACOS_X64"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/honeycomb-Technologies/Krusty/releases/download/vVERSION_PLACEHOLDER/krusty-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "SHA256_LINUX_ARM64"
    else
      url "https://github.com/honeycomb-Technologies/Krusty/releases/download/vVERSION_PLACEHOLDER/krusty-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "SHA256_LINUX_X64"
    end
  end

  def install
    bin.install "krusty"
    bin.install "krusty-mako"
  end

  test do
    assert_match "krusty", shell_output("#{bin}/krusty --help")
    assert_match "krusty-mako", shell_output("#{bin}/krusty-mako --version")
  end

  service do
    run [opt_bin/"krusty-mako", "daemon"]
    keep_alive true
    working_dir var
    log_path var/"log/krusty-mako.log"
    error_log_path var/"log/krusty-mako.log"
    # launchd does not inherit the interactive shell PATH. Preserve Homebrew's
    # standard service path so daemon-owned coding tools remain discoverable.
    environment_variables PATH: std_service_path_env,
                          RUST_LOG: "krusty_mako=info"
  end
end
