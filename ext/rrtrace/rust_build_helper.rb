# frozen_string_literal: true

require "fileutils"
require "rbconfig"

module RustBuildHelper
  def self.build(root_dir)
    puts "--- Building Rust visualizer ---"
    cargo_toml = File.join(root_dir, "Cargo.toml")

    unless File.exist?(cargo_toml)
      puts "Cargo.toml not found at #{cargo_toml}. Skipping rust build (maybe pre-compiled gem?)"
      return
    end

    unless system("cargo --version")
      puts "Cargo not found. Please install Rust to build this gem from source."
      exit 1
    end

    # Build rust binary
    # We use --release for performance of the visualizer
    sh_cmd = "cargo build --release --locked"
    puts "Running: #{sh_cmd} in #{root_dir}"
    unless system(sh_cmd, chdir: root_dir)
      puts "Failed to build Rust visualizer."
      exit 1
    end

    # Copy to libexec
    exe = "rrtrace#{RbConfig::CONFIG["EXEEXT"]}"
    src_exe = File.join(root_dir, "target", "release", exe)
    dest_dir = File.join(root_dir, "libexec")
    FileUtils.mkdir_p(dest_dir)
    dest_exe = File.join(dest_dir, exe)

    puts "Copying #{src_exe} to #{dest_exe}"
    FileUtils.cp(src_exe, dest_exe)
    FileUtils.chmod(0755, dest_exe) unless Gem.win_platform?
  end
end
