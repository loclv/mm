class Mm < Formula
  desc "Generate a Conventional Commit message from Git changes and commit it"
  homepage "https://github.com/loclv/mm"
  version "0.2.2"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/loclv/mm/releases/download/release-v0.2.2/mm-aarch64-apple-darwin.tar.gz"
      # sha256 will be updated by release pipeline
    end
    on_intel do
      url "https://github.com/loclv/mm/releases/download/release-v0.2.2/mm-x86_64-apple-darwin.tar.gz"
      # sha256 will be updated by release pipeline
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/loclv/mm/releases/download/release-v0.2.2/mm-aarch64-unknown-linux-gnu.tar.gz"
      # sha256 will be updated by release pipeline
    end
    on_intel do
      url "https://github.com/loclv/mm/releases/download/release-v0.2.2/mm-x86_64-unknown-linux-gnu.tar.gz"
      # sha256 will be updated by release pipeline
    end
  end

  def install
    bin.install "mm"
  end

  test do
    assert_match "Generate a Conventional Commit message", shell_output("#{bin}/mm --help")
  end
end
