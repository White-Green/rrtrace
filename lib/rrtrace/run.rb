# frozen_string_literal: true

require "rrtrace"

Rrtrace.start
at_exit { Rrtrace.stop }
