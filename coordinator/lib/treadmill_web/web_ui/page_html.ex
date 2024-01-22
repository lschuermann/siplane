defmodule TreadmillWeb.WebUI.PageHTML do
  use TreadmillWeb, :html

  embed_templates "page_html/*"

  def get_boards() do
    # add :id, :binary_id, primary_key: true, null: false
    #   add :label, :string, null: false
    #   add :manufacturer, :string, null: false
    #   add :model, :string, null: false
    #   add :image_url, :string, null: true
    #   add :hwrev, :string, null: true
    #   add :location, :string, null: false
    #   add :runner_token, :string, null: false
    [%{
	label: "nRF54820DK Princeton",
	manufacturer: "Nordic Semiconductor",
	model: "nRF52840DK",
	image_url: "https://www.nordicsemi.com/-/media/Images/Products/DevKits/nRF52-Series/nRF52840-DK/nRF52840-DK-promo.png?sc_lang=en",
	hwrev: "foobar",
	location: "Princeton University",
	runner_connected: false,
     }]
    # [1, 2, 3]
    # Treadmill.
  end
end
