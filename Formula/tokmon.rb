class Tokmon < Formula
  desc "htop for your AI spend — track tokens, latency, and cost across LLM providers"
  homepage "https://github.com/trgr/tokmon"
  url "https://github.com/trgr/tokmon/archive/refs/tags/v0.2.0.tar.gz"
  sha256 "6a0bb870e897168cdff10ef32f00bf97be86d48f3c87424595a542a77b39cae4"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "tokmon", shell_output("#{bin}/tokmon --help")
  end
end
