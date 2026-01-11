# frozen_string_literal: true

require_relative "lib/rrtrace/version"

Gem::Specification.new do |spec|
  spec.name = "rrtrace"
  spec.version = Rrtrace::VERSION
  spec.authors = ["White-Green"]
  spec.email = ["43771790+White-Green@users.noreply.github.com"]

  spec.summary = "A Ruby trace tool with Rust-based visualizer"
  spec.description = "Rrtrace is a profiling tool that captures Ruby method calls and visualizes them using a high-performance Rust renderer."
  spec.homepage = "https://github.com/White-Green/rrtrace"
  spec.license = "MIT"
  spec.required_ruby_version = ">= 3.2.0"

  spec.metadata["homepage_uri"] = spec.homepage

  # Specify which files should be added to the gem when it is released.
  # The `git ls-files -z` loads the files in the RubyGem that have been added into git.
  gemspec = File.basename(__FILE__)
  spec.files = IO.popen(%w[git ls-files -z], chdir: __dir__, err: IO::NULL) do |ls|
    ls.readlines("\x0", chomp: true).reject do |f|
      (f == gemspec) ||
        f.start_with?(*%w[bin/ Gemfile .gitignore .standard.yml])
    end
  end

  # Include Rust source for 'ruby' platform gem
  # But exclude it for native gems
  if spec.platform == Gem::Platform::RUBY
    spec.files += Dir["Cargo.*", "src/**/*"]
    spec.extensions = ["ext/rrtrace/extconf.rb"]
  else
    spec.extensions = []
  end

  spec.files += Dir["libexec/*"]
  spec.bindir = "exe"
  spec.executables = spec.files.grep(%r{\Aexe/}) { |f| File.basename(f) }
  spec.require_paths = ["lib"]

  spec.add_development_dependency "rake-compiler"
end
