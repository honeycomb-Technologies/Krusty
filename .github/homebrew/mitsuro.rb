# Homebrew formula for Mitsuro
# To use: brew install BurgessTG/tap/mitsuro
#
# Release CI renders this template and attaches mitsuro.rb to the GitHub Release.
# Publishing Formula/mitsuro.rb to BurgessTG/homebrew-tap is a separate maintainer
# step unless an explicitly configured, repository-scoped token enables it.

class Mitsuro < Formula
  desc "Mitsuro multi-platform AI coding product with Agent and Hive"
  homepage "https://github.com/honeycomb-Technologies/Mitsuro"
  version "VERSION_PLACEHOLDER"
  license "MIT"
  conflicts_with "krusty", because: "both install the same compatibility commands"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/honeycomb-Technologies/Mitsuro/releases/download/vVERSION_PLACEHOLDER/mitsuro-aarch64-apple-darwin.tar.gz"
      sha256 "SHA256_MACOS_ARM64"
    else
      url "https://github.com/honeycomb-Technologies/Mitsuro/releases/download/vVERSION_PLACEHOLDER/mitsuro-x86_64-apple-darwin.tar.gz"
      sha256 "SHA256_MACOS_X64"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/honeycomb-Technologies/Mitsuro/releases/download/vVERSION_PLACEHOLDER/mitsuro-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "SHA256_LINUX_ARM64"
    else
      url "https://github.com/honeycomb-Technologies/Mitsuro/releases/download/vVERSION_PLACEHOLDER/mitsuro-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "SHA256_LINUX_X64"
    end
  end

  def install
    bin.install "mitsuro"
    bin.install "mitsuro-hive"
    bin.install "krusty"
    bin.install "krusty-mako"
  end

  test do
    canonical_cli = shell_output("#{bin}/mitsuro --version").strip
    canonical_hive = shell_output("#{bin}/mitsuro-hive --version").strip
    assert_equal canonical_cli, shell_output("#{bin}/krusty --version").strip
    assert_equal canonical_hive, shell_output("#{bin}/krusty-mako --version").strip
  end

  service do
    run [opt_bin/"mitsuro-hive", "daemon"]
    keep_alive true
    working_dir var
    log_path var/"log/mitsuro-hive.log"
    error_log_path var/"log/mitsuro-hive.log"
    # launchd does not inherit the interactive shell PATH. Preserve Homebrew's
    # standard service path so daemon-owned coding tools remain discoverable.
    environment_variables PATH: std_service_path_env,
                          RUST_LOG: "mitsuro_hive=info"
  end
end
