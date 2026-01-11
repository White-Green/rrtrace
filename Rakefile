# frozen_string_literal: true

require "bundler/gem_tasks"
require "standard/rake"

require "rake/extensiontask"

task build: :compile

GEMSPEC = Gem::Specification.load("rrtrace.gemspec")

Rake::ExtensionTask.new("rrtrace", GEMSPEC) do |ext|
  ext.lib_dir = "lib/rrtrace"
end

task :build_rust do
  GEM_ROOT = File.expand_path(__dir__)
  OUT_DIR  = File.join(GEM_ROOT, "libexec")
  exe = "rrtrace#{RbConfig::CONFIG["EXEEXT"]}"
  dest_exe_path = File.join(OUT_DIR, exe)
  FileUtils.mkdir_p(OUT_DIR)
  sh "cargo", "build", "--release", "--locked"
  FileUtils.cp(File.join("target", "release", exe), dest_exe_path)
  FileUtils.chmod("+x", dest_exe_path) unless Gem.win_platform?
end

Rake::Task["compile"].enhance([:build_rust])

task default: %i[clobber compile standard]
