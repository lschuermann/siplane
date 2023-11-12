defmodule Siplane.Repo do
  use Ecto.Repo,
    otp_app: :siplane,
    adapter: Ecto.Adapters.Postgres
end
