defmodule Treadmill.Repo do
  use Ecto.Repo,
    otp_app: :treadmill,
    adapter: Ecto.Adapters.Postgres
end
