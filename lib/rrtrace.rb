# frozen_string_literal: true

require "rbconfig"

require_relative "rrtrace/version"

module Rrtrace
  class << self
    def visualizer_path
      @visualizer_path ||= default_visualizer_path
    end

    def start
      native_start(visualizer_path)
    end

    def stop
      native_stop
    end

    def started?
      native_started?
    end

    private

    def default_visualizer_path
      exe = "rrtrace#{RbConfig::CONFIG["EXEEXT"]}"
      File.expand_path("../libexec/#{exe}", __dir__)
    end
  end
end

require "rrtrace/rrtrace"

module Rrtrace
  private_class_method :native_start, :native_stop, :native_started?
end
