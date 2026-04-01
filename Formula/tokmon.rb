class Tokmon < Formula
  desc "htop for your AI spend — track tokens, latency, and cost across LLM providers"
  homepage "https://github.com/trgr/tokmon"
  url "https://github.com/trgr/tokmon/archive/refs/tags/v0.3.0.tar.gz"
  sha256 "e5821b700a8c9d543267cd8d609e42d8c3a8b86e12ec8d1923814370903cc138"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "tokmon", shell_output("#{bin}/tokmon --help")
  end
end
