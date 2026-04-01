class Tokmon < Formula
  desc "htop for your AI spend — track tokens, latency, and cost across LLM providers"
  homepage "https://github.com/trgr/tokmon"
  url "https://github.com/trgr/tokmon/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "4d78aefc5779ab4640626088856034cda920c92dc84f3258af7de7f66b56f488"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "tokmon", shell_output("#{bin}/tokmon --help")
  end
end
