class Mm < Formula
  desc "Generate a Conventional Commit message from Git changes and commit it"
  homepage "https://github.com/loclv/mm"
  version "0.2.10"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/loclv/mm/releases/download/release-v0.2.10/mm-aarch64-apple-darwin.tar.gz"
      sha256 "825de8c80029eb61a28a9c85ab451d193bf0862884d609f96c8414e7aa6b8076"
    end
    on_intel do
      url "https://github.com/loclv/mm/releases/download/release-v0.2.10/mm-x86_64-apple-darwin.tar.gz"
      sha256 "582a6d2274eb6dba0e438e7582e73b5dd37f3e31b4a1720e28d4b36c9be7b8ef"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/loclv/mm/releases/download/release-v0.2.10/mm-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "915dfd5ed57d492e6a96d7f7103fced19806990e0ff50aa853066c50b1a9e18c"
    end
    on_intel do
      url "https://github.com/loclv/mm/releases/download/release-v0.2.10/mm-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "fd080eb00b16f8fdc5511bd9872783105f0c28a3f142b87edaf4a877e6b143eb"
    end
  end

  def install
    bin.install "mm"
  end

  test do
    assert_match "mm - commit with an automatically generated message", shell_output("#{bin}/mm --help")
  end
end
