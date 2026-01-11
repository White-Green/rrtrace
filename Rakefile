# frozen_string_literal: true

require "bundler/gem_tasks"
require "standard/rake"

require "rake/extensiontask"

task build: :compile

GEMSPEC = Gem::Specification.load("rrtrace.gemspec")

Rake::ExtensionTask.new("rrtrace", GEMSPEC) do |ext|
  ext.lib_dir = "lib/rrtrace"
end

task default: %i[clobber compile standard]
