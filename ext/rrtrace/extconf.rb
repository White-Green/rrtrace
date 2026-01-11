# frozen_string_literal: true

require "mkmf"
require "fileutils"

require_relative "rust_build_helper"

# Makes all symbols private by default to avoid unintended conflict
# with other gems. To explicitly export symbols you can use RUBY_FUNC_EXPORTED
# selectively, or entirely remove this flag.
append_cflags("-fvisibility=hidden")

root_dir = File.expand_path("../../", __dir__)
RustBuildHelper.build(root_dir)

create_makefile("rrtrace/rrtrace")
