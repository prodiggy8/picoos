import numpy as np

# Parameters
T = 1000
t_target = 100
beta_start = 0.0001
beta_end = 0.02

# 1. Generate the beta values for the first 100 steps (indices 0 to 99)
# Formula: beta_t = 0.0001 + (0.02 - 0.0001) * t / 1000
t_indices = np.arange(t_target) # Generates 0, 1, ..., 99
betas = beta_start + (beta_end - beta_start) * t_indices / 1000

# 2. Calculate alphas
alphas = 1 - betas

# 3. Calculate alpha_bar (cumulative product)
alpha_bar_t = np.prod(alphas)

# 4. Calculate variance
variance = 1 - alpha_bar_t

print(f"Variance: {variance}")

# parameters
T = 1000
t = 100
x0 = 5

betas = 0.0001 + (0.02 - 0.0001) * np.arange(t) / T
alphas = 1 - betas
alpha_bar_t = np.prod(alphas[:t+1])

variance = 1 - alpha_bar_t
print(variance)
